mod api;
mod auth;
mod checkers;
mod config;
mod db;
mod error;
mod notifications;
mod scheduler;
mod state;
mod web;

use clap::Parser;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, watch};
use tracing::info;

use crate::config::{AppConfig, Cli, Commands};
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kernelci_status=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let cfg = AppConfig::load(cli).await;

    // Ensure parent directory exists for the database
    if let Some(parent) = std::path::Path::new(&cfg.db_path).parent() {
        if !parent.as_os_str().is_empty() {
            match tokio::fs::metadata(parent).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    tokio::fs::create_dir_all(parent).await?;
                    info!("Created database directory: {}", parent.display());
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    let conn = db::open_and_migrate(&cfg.db_path).await?;

    // Handle create-user subcommand
    if let Some(Commands::CreateUser { username }) = &cfg.command {
        let username = username.clone();
        eprintln!("Enter password for user '{username}': ");
        let mut password = String::new();
        std::io::stdin().read_line(&mut password)?;
        let password = password.trim().to_string();

        if password.is_empty() {
            eprintln!("Password cannot be empty");
            std::process::exit(1);
        }

        let hash = auth::password::hash_password(&password)
            .map_err(|e| format!("Failed to hash password: {e}"))?;

        conn.call(move |conn| -> rusqlite::Result<()> {
            db::users::insert(conn, &username, &hash)?;
            Ok(())
        })
        .await?;

        eprintln!("User created successfully");
        return Ok(());
    }

    // Create default user from config if no users exist
    if let (Some(username), Some(password)) = (&cfg.default_username, &cfg.default_password) {
        let username = username.clone();
        let password = password.clone();

        let user_count: i64 = conn
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0)))
            .await?;

        if user_count == 0 {
            let hash = auth::password::hash_password(&password)
                .map_err(|e| format!("Failed to hash password: {e}"))?;
            let u = username.clone();
            conn.call(move |conn| -> rusqlite::Result<()> {
                db::users::insert(conn, &u, &hash)?;
                Ok(())
            })
            .await?;
            info!("Created default user '{username}' from config file");
        }
    }

    // Load config cache
    let config_cache: HashMap<String, String> = conn
        .call(|conn| -> rusqlite::Result<_> {
            let pairs = db::config::get_all(conn)?;
            Ok(pairs.into_iter().collect())
        })
        .await?;

    // Notification channel
    let (notify_tx, notify_rx) = mpsc::channel(100);

    let secure_cookies = cfg.acme.is_some();
    let app_state = AppState {
        db: conn.clone(),
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?,
        config_cache: Arc::new(RwLock::new(config_cache)),
        notify_tx,
        secure_cookies,
    };

    // Shutdown signal
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Start notification worker
    let notif_state = app_state.clone();
    tokio::spawn(notifications::notification_worker(
        notify_rx,
        notif_state.db,
        notif_state.http_client,
    ));

    // Start scheduler
    let sched_state = app_state.clone();
    let sched_shutdown = shutdown_rx.clone();
    tokio::spawn(scheduler::run(sched_state, sched_shutdown));

    // Build router
    let app = web::router(app_state);

    // Ctrl+C task — owns shutdown_tx so it can notify the scheduler/other workers.
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("Shutdown signal received");
        let _ = shutdown_tx.send(true);
    });

    if let Some(acme_cfg) = cfg.acme {
        serve_with_acme(app, acme_cfg).await?;
    } else {
        let addr = format!("0.0.0.0:{}", cfg.port);
        info!("Starting server on {addr}");
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                tokio::signal::ctrl_c()
                    .await
                    .expect("Failed to listen for Ctrl+C");
            })
            .await?;
    }

    info!("Server stopped");
    Ok(())
}

/// Serve the app over HTTPS on `https_port` with an auto-renewing Let's Encrypt
/// certificate (via the TLS-ALPN-01 challenge), and a plain-HTTP listener on
/// `http_port` that 301-redirects everything to HTTPS.
async fn serve_with_acme(
    app: axum::Router,
    cfg: crate::config::AcmeConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    use futures_util::StreamExt;
    use rustls_acme::{AcmeConfig, caches::DirCache};
    use std::net::SocketAddr;

    // Cache dir must exist and be writable — rustls-acme stores the account
    // key and issued certs here, which is what makes renewal automatic across
    // restarts (it will reuse a cert that still has >30d of validity, and
    // renew in-place when it gets close to expiry).
    tokio::fs::create_dir_all(&cfg.cache_dir).await?;

    info!(
        "ACME enabled: domains={:?}, staging={}, cache_dir={}",
        cfg.domains, cfg.staging, cfg.cache_dir
    );

    let mut acme_state = AcmeConfig::new(cfg.domains.clone())
        .cache(DirCache::new(cfg.cache_dir.clone()))
        .directory_lets_encrypt(!cfg.staging);
    if let Some(contact) = &cfg.contact {
        // rustls-acme expects the full `mailto:` URI.
        let uri = if contact.starts_with("mailto:") {
            contact.clone()
        } else {
            format!("mailto:{contact}")
        };
        acme_state = acme_state.contact_push(uri);
    }
    let mut acme_state = acme_state.state();
    let rustls_config = acme_state.default_rustls_config();
    let acceptor = acme_state.axum_acceptor(rustls_config);

    // Drive the ACME state machine: this is what performs issuance and
    // background renewal. Without polling this stream the cert is never
    // obtained (or renewed).
    tokio::spawn(async move {
        loop {
            match acme_state.next().await {
                Some(Ok(ok)) => tracing::info!("acme event: {ok:?}"),
                Some(Err(err)) => tracing::error!("acme error: {err:?}"),
                None => {
                    tracing::warn!("acme state stream ended");
                    break;
                }
            }
        }
    });

    // Port 80: redirect everything to HTTPS. Also works as a sanity-check
    // endpoint from outside that the daemon is reachable on 80 — LE does not
    // use HTTP-01 here (we use TLS-ALPN-01 on 443), but operators and probes
    // often hit 80 first.
    let http_addr: SocketAddr = ([0, 0, 0, 0], cfg.http_port).into();
    let https_port = cfg.https_port;
    let redirect_app =
        axum::Router::new().fallback(move |req: axum::http::Request<axum::body::Body>| {
            let https_port = https_port;
            async move { redirect_to_https(req, https_port) }
        });
    tokio::spawn(async move {
        info!("Starting HTTP redirect listener on {http_addr}");
        match tokio::net::TcpListener::bind(&http_addr).await {
            Ok(listener) => {
                if let Err(e) = axum::serve(listener, redirect_app)
                    .with_graceful_shutdown(async {
                        let _ = tokio::signal::ctrl_c().await;
                    })
                    .await
                {
                    tracing::error!("HTTP redirect listener error: {e}");
                }
            }
            Err(e) => tracing::error!("Failed to bind HTTP port {}: {e}", cfg.http_port),
        }
    });

    // Port 443: axum-server with the ACME acceptor.
    let https_addr: SocketAddr = ([0, 0, 0, 0], cfg.https_port).into();
    info!("Starting HTTPS listener on {https_addr}");
    axum_server::bind(https_addr)
        .acceptor(acceptor)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

fn redirect_to_https(
    req: axum::http::Request<axum::body::Body>,
    https_port: u16,
) -> axum::response::Response {
    use axum::http::{StatusCode, header};
    use axum::response::{IntoResponse, Response};

    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or(h).to_string());

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");

    match host {
        Some(host) => {
            let location = if https_port == 443 {
                format!("https://{host}{path_and_query}")
            } else {
                format!("https://{host}:{https_port}{path_and_query}")
            };
            let mut resp = Response::new(axum::body::Body::empty());
            *resp.status_mut() = StatusCode::MOVED_PERMANENTLY;
            if let Ok(val) = axum::http::HeaderValue::from_str(&location) {
                resp.headers_mut().insert(header::LOCATION, val);
            }
            resp
        }
        None => (StatusCode::BAD_REQUEST, "missing Host header").into_response(),
    }
}
