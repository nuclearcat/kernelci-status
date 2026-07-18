// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use crate::checkers::{CheckContext, CheckResult, EndpointState};
use crate::db::endpoints::Endpoint;
use std::time::{Duration, Instant};

/// Number of requests per latency check; the median is reported so a couple
/// of slow responses don't trip the alert condition. Must stay small enough
/// that all samples fit within the scheduler's per-check timeout (30s).
const LATENCY_SAMPLES: usize = 5;

/// Pause between latency samples.
const SAMPLE_DELAY: Duration = Duration::from_millis(100);

/// Timeout for each individual latency sample, overriding the client-wide
/// 30s timeout so all samples fit within the scheduler's per-check timeout.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(5);

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
    let mut states: Vec<EndpointState> = Vec::with_capacity(LATENCY_SAMPLES);
    let mut latencies: Vec<u128> = Vec::with_capacity(LATENCY_SAMPLES);
    let mut statuses: Vec<u16> = Vec::with_capacity(LATENCY_SAMPLES);
    let mut last_error: Option<String> = None;

    for i in 0..LATENCY_SAMPLES {
        if i > 0 {
            tokio::time::sleep(SAMPLE_DELAY).await;
        }

        let start = Instant::now();
        match ctx
            .http_client
            .get(&endpoint.endpoint)
            .timeout(SAMPLE_TIMEOUT)
            .send()
            .await
        {
            Ok(resp) => {
                latencies.push(start.elapsed().as_millis());
                let status = resp.status();
                statuses.push(status.as_u16());
                states.push(if status.is_server_error() {
                    EndpointState::Critical
                } else if !status.is_success() {
                    EndpointState::Warning
                } else {
                    EndpointState::Ok
                });
            }
            Err(e) => {
                states.push(EndpointState::Critical);
                last_error = Some(format!("HTTP request failed: {e}"));
            }
        }
    }

    if latencies.is_empty() {
        return Err(last_error.unwrap_or_else(|| "HTTP request failed".to_string()));
    }

    // Median across all samples (failed requests count as Critical), so one
    // transient error or slow response doesn't flip the endpoint state.
    states.sort();
    let state = states[states.len() / 2].clone();
    let latency_ms = median(&mut latencies);

    statuses.sort_unstable();
    let status = statuses[statuses.len() / 2];

    Ok(CheckResult {
        state,
        value: Some(latency_ms.to_string()),
        message: Some(format!(
            "HTTP {status}, latency: {latency_ms}ms (median of {}/{} samples)",
            latencies.len(),
            LATENCY_SAMPLES
        )),
    })
}

/// Median of the collected samples; for an even count, the mean of the two
/// middle values.
fn median(samples: &mut [u128]) -> u128 {
    samples.sort_unstable();
    let mid = samples.len() / 2;
    if samples.len() % 2 == 0 {
        (samples[mid - 1] + samples[mid]) / 2
    } else {
        samples[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_median_odd() {
        assert_eq!(median(&mut [50, 900, 60]), 60);
        assert_eq!(median(&mut [100]), 100);
    }

    #[test]
    fn test_median_even() {
        assert_eq!(median(&mut [40, 60]), 50);
        assert_eq!(median(&mut [10, 20, 900, 30]), 25);
    }
}
