// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use crate::checkers::{CheckContext, CheckResult, EndpointState};
use crate::db::endpoints::Endpoint;
use openssh::{KnownHosts, Session};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CMD_TIMEOUT: Duration = Duration::from_secs(15);

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
    // endpoint format: ssh://user@host or ssh://host
    let url = &endpoint.endpoint;
    let host = url.strip_prefix("ssh://").unwrap_or(url);

    let session = tokio::time::timeout(CONNECT_TIMEOUT, Session::connect(host, KnownHosts::Accept))
        .await
        .map_err(|_| "SSH connection timed out (10s)".to_string())?
        .map_err(|e| format!("SSH connection failed: {e}"))?;

    let output = tokio::time::timeout(
        CMD_TIMEOUT,
        session
            .command("docker")
            .arg("ps")
            .arg("--format")
            .arg("{{.Names}}\t{{.Status}}")
            .output(),
    )
    .await
    .map_err(|_| "docker ps timed out (15s)".to_string())?
    .map_err(|e| format!("docker ps failed: {e}"))?;

    if !output.status.success() {
        return Err(format!("docker ps exited with {}", output.status));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let container_filter = endpoint.selector.as_deref().unwrap_or("");

    let mut warnings = Vec::new();
    let mut total = 0;

    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0];
        let status = parts[1];

        if !container_filter.is_empty() && !name.contains(container_filter) {
            continue;
        }

        total += 1;

        if let Some(minutes) = parse_uptime_minutes(status) {
            if minutes < 30 {
                warnings.push(format!("{name}: uptime {minutes}m"));
            }
        }
    }

    // Best-effort close, don't hang if remote is unresponsive
    let _ = tokio::time::timeout(Duration::from_secs(5), session.close()).await;

    if !warnings.is_empty() {
        Ok(CheckResult {
            state: EndpointState::Warning,
            value: Some(total.to_string()),
            message: Some(format!("Low uptime: {}", warnings.join(", "))),
        })
    } else {
        Ok(CheckResult {
            state: EndpointState::Ok,
            value: Some(total.to_string()),
            message: Some(format!("{total} containers running")),
        })
    }
}

fn parse_uptime_minutes(status: &str) -> Option<i64> {
    let status = status.to_lowercase();
    if !status.contains("up") {
        return Some(0);
    }

    if status.contains("second") {
        return Some(0);
    }

    let parts: Vec<&str> = status.split_whitespace().collect();
    for (i, part) in parts.iter().enumerate() {
        if let Ok(n) = part.parse::<i64>() {
            if let Some(unit) = parts.get(i + 1) {
                if unit.starts_with("minute") {
                    return Some(n);
                } else if unit.starts_with("hour") {
                    return Some(n * 60);
                } else if unit.starts_with("day") {
                    return Some(n * 24 * 60);
                } else if unit.starts_with("week") {
                    return Some(n * 7 * 24 * 60);
                }
            }
        }
    }

    if status.contains("about an hour") {
        return Some(60);
    }
    if status.contains("about a minute") {
        return Some(1);
    }

    None
}
