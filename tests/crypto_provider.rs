// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

//! Regression tests for rustls CryptoProvider (see rustls 0.23+ requirement).
//!
//! Without a process-level CryptoProvider installed, any TLS operation panics:
//!   "Could not automatically determine the process-level CryptoProvider"
//!
//! The application installs it in main(). These tests verify the provider
//! works correctly so we catch breakage before deployment.

fn install_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[test]
fn rustls_client_config_does_not_panic() {
    install_provider();

    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let _config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
}

#[test]
fn reqwest_client_builds_with_rustls() {
    install_provider();

    let _client = reqwest::Client::builder()
        .use_rustls_tls()
        .build()
        .expect("reqwest client with rustls should build without panic");
}

#[tokio::test]
async fn tls_connector_accepts_connections() {
    install_provider();

    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    // Just verify the connector can be constructed without panic
    let _connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(tls_config));
}
