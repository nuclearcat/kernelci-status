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
    match endpoint.check_type.as_str() {
        "http_status" => http::check(endpoint, ctx).await,
        "http_latency" => http::check_latency(endpoint, ctx).await,
        "tls_cert" => tls::check(endpoint, ctx).await,
        "prometheus" => prometheus::check(endpoint, ctx).await,
        "postgresql" => postgresql::check(endpoint, ctx).await,
        "kubernetes" => kubernetes::check(endpoint, ctx).await,
        "docker" => docker::check(endpoint, ctx).await,
        other => CheckResult {
            state: EndpointState::NoData,
            value: None,
            message: Some(format!("Unknown check_type: {other}")),
        },
    }
}
