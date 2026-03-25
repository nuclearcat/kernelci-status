pub mod condition;
pub mod docker;
pub mod http;
pub mod kubernetes;
pub mod postgresql;
pub mod prometheus;
pub mod tls;

use crate::db::endpoints::Endpoint;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum EndpointState {
    Ok,
    NoData,
    Warning,
    Critical,
}

impl fmt::Display for EndpointState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EndpointState::Ok => write!(f, "OK"),
            EndpointState::NoData => write!(f, "NO_DATA"),
            EndpointState::Warning => write!(f, "WARNING"),
            EndpointState::Critical => write!(f, "CRITICAL"),
        }
    }
}



#[derive(Debug, Clone)]
pub struct CheckResult {
    pub state: EndpointState,
    pub value: Option<String>,
    pub message: Option<String>,
}

pub struct CheckContext {
    pub http_client: reqwest::Client,
}

pub async fn dispatch_check(endpoint: &Endpoint, ctx: &CheckContext) -> CheckResult {
    let url = &endpoint.endpoint;

    let result = if url.starts_with("http://") || url.starts_with("https://") {
        match endpoint.selector.as_deref() {
            Some("cert_expiration") => tls::check(endpoint, ctx).await,
            Some("latency") => http::check_latency(endpoint, ctx).await,
            _ => http::check(endpoint, ctx).await,
        }
    } else if url.starts_with("promhttp://") || url.starts_with("promhttps://") {
        prometheus::check(endpoint, ctx).await
    } else if url.starts_with("postgresql://") {
        postgresql::check(endpoint, ctx).await
    } else if url.starts_with("k8s://") {
        kubernetes::check(endpoint, ctx).await
    } else if url.starts_with("ssh://") {
        docker::check(endpoint, ctx).await
    } else {
        CheckResult {
            state: EndpointState::NoData,
            value: None,
            message: Some(format!("Unknown endpoint scheme: {url}")),
        }
    };

    result
}
