use crate::checkers::{CheckContext, CheckResult, EndpointState};
use crate::db::endpoints::Endpoint;
use rustls_pki_types::ServerName;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use x509_parser::prelude::*;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn check(endpoint: &Endpoint, _ctx: &CheckContext) -> CheckResult {
    match do_check(endpoint).await {
        Ok(r) => r,
        Err(e) => CheckResult {
            state: EndpointState::Critical,
            value: None,
            message: Some(e),
        },
    }
}

async fn do_check(endpoint: &Endpoint) -> Result<CheckResult, String> {
    let url: url::Url = endpoint
        .endpoint
        .parse()
        .map_err(|e| format!("Invalid URL: {e}"))?;

    let host = url.host_str().ok_or("No host in URL")?;
    let port = url.port().unwrap_or(443);
    let addr = format!("{host}:{port}");

    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(tls_config));
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| format!("Invalid server name: {e}"))?;

    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map_err(|_| "TCP connect timed out (10s)".to_string())?
        .map_err(|e| format!("TCP connect failed: {e}"))?;

    let tls_stream = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(server_name, stream))
        .await
        .map_err(|_| "TLS handshake timed out (10s)".to_string())?
        .map_err(|e| format!("TLS handshake failed: {e}"))?;

    let (_io, session) = tls_stream.into_inner();
    let certs = session
        .peer_certificates()
        .ok_or("No peer certificates")?;

    let leaf_cert = certs.first().ok_or("No leaf certificate")?;

    let (_, cert) = X509Certificate::from_der(leaf_cert.as_ref())
        .map_err(|e| format!("Failed to parse certificate: {e}"))?;

    let not_after = cert.validity().not_after.to_datetime();
    let not_after_ts = not_after.unix_timestamp();
    let now_ts = chrono::Utc::now().timestamp();
    let days_remaining = (not_after_ts - now_ts) / 86400;

    let state = if days_remaining < 0 {
        EndpointState::Critical
    } else if days_remaining < 7 {
        EndpointState::Warning
    } else {
        EndpointState::Ok
    };

    Ok(CheckResult {
        state,
        value: Some(days_remaining.to_string()),
        message: Some(format!("Certificate expires in {days_remaining} days")),
    })
}
