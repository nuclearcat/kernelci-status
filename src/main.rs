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
use tokio::sync::{mpsc, watch, RwLock};
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
    let cfg = AppConfig::load(cli);

    // Ensure parent directory exists for the database
    if let Some(parent) = std::path::Path::new(&cfg.db_path).parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)?;
            info!("Created database directory: {}", parent.display());
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
            .call(|conn| {
                conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            })
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

    let app_state = AppState {
        db: conn.clone(),
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?,
        config_cache: Arc::new(RwLock::new(config_cache)),
        notify_tx,
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

    let addr = format!("0.0.0.0:{}", cfg.port);
    info!("Starting server on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to listen for Ctrl+C");
            info!("Shutdown signal received");
            let _ = shutdown_tx.send(true);
        })
        .await?;

    info!("Server stopped");
    Ok(())
}
