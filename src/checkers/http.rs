use crate::checkers::{CheckContext, CheckResult, EndpointState};
use crate::db::endpoints::Endpoint;
use std::time::Instant;

pub async fn check(endpoint: &Endpoint, ctx: &CheckContext) -> CheckResult {
    match do_check(endpoint, ctx).await {
        Ok(r) => r,
        Err(e) => CheckResult {
            state: EndpointState::Critical,
            value: None,
            message: Some(e),
        },
    }
}

async fn do_check(endpoint: &Endpoint, ctx: &CheckContext) -> Result<CheckResult, String> {
    let resp = ctx
        .http_client
        .get(&endpoint.endpoint)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    let status = resp.status().as_u16();
    let state = if resp.status().is_success() {
        EndpointState::Ok
    } else if resp.status().is_server_error() {
        EndpointState::Critical
    } else {
        EndpointState::Warning
    };

    Ok(CheckResult {
        state,
        value: Some(status.to_string()),
        message: Some(format!("HTTP {status}")),
    })
}

pub async fn check_latency(endpoint: &Endpoint, ctx: &CheckContext) -> CheckResult {
    match do_check_latency(endpoint, ctx).await {
        Ok(r) => r,
        Err(e) => CheckResult {
            state: EndpointState::Critical,
            value: None,
            message: Some(e),
        },
    }
}

async fn do_check_latency(endpoint: &Endpoint, ctx: &CheckContext) -> Result<CheckResult, String> {
    let start = Instant::now();
    let resp = ctx
        .http_client
        .get(&endpoint.endpoint)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;
    let latency_ms = start.elapsed().as_millis();

    let status = resp.status().as_u16();
    let state = if resp.status().is_server_error() {
        EndpointState::Critical
    } else if !resp.status().is_success() {
        EndpointState::Warning
    } else {
        EndpointState::Ok
    };

    Ok(CheckResult {
        state,
        value: Some(latency_ms.to_string()),
        message: Some(format!("HTTP {status}, latency: {latency_ms}ms")),
    })
}
