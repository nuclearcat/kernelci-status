// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use crate::checkers::{CheckContext, CheckResult, EndpointState};
use crate::db::endpoints::Endpoint;
use k8s_openapi::api::core::v1::{ContainerStatus, Pod};
use kube::{Api, Client, Config};
use std::time::Duration;

const API_TIMEOUT: Duration = Duration::from_secs(15);
const RECENT_RESTART_WINDOW_SECS: i64 = 30 * 60;

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
    let namespace = &endpoint.endpoint;
    let label_selector = endpoint.selector.as_deref();

    let config = tokio::time::timeout(API_TIMEOUT, Config::infer())
        .await
        .map_err(|_| "Kubeconfig load timed out (15s)".to_string())?
        .map_err(|e| format!("Failed to load kubeconfig: {e}"))?;
    let client =
        Client::try_from(config).map_err(|e| format!("Failed to create k8s client: {e}"))?;

    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let mut lp = kube::api::ListParams::default();
    if let Some(sel) = label_selector {
        lp = lp.labels(sel);
    }

    let pod_list = tokio::time::timeout(API_TIMEOUT, pods.list(&lp))
        .await
        .map_err(|_| "K8s API request timed out (15s)".to_string())?
        .map_err(|e| format!("Failed to list pods: {e}"))?;

    let now_ts = chrono::Utc::now().timestamp();
    let total_pods = pod_list.items.len();
    let mut total_restarts: i64 = 0;
    let mut criticals: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for pod in &pod_list.items {
        let pod_name = pod.metadata.name.as_deref().unwrap_or("unknown");
        let Some(status) = &pod.status else {
            continue;
        };

        let containers = status
            .container_statuses
            .iter()
            .flatten()
            .chain(status.init_container_statuses.iter().flatten());

        for cs in containers {
            total_restarts += cs.restart_count as i64;
            inspect_container(pod_name, cs, now_ts, &mut criticals, &mut warnings);
        }
    }

    let value = Some(total_restarts.to_string());

    if !criticals.is_empty() {
        return Ok(CheckResult {
            state: EndpointState::Critical,
            value,
            message: Some(criticals.join("; ")),
        });
    }
    if !warnings.is_empty() {
        return Ok(CheckResult {
            state: EndpointState::Warning,
            value,
            message: Some(format!("Recent restarts: {}", warnings.join(", "))),
        });
    }

    Ok(CheckResult {
        state: EndpointState::Ok,
        value,
        message: Some(format!(
            "{total_pods} pods, {total_restarts} total restarts, no recent restarts"
        )),
    })
}

fn inspect_container(
    pod_name: &str,
    cs: &ContainerStatus,
    now_ts: i64,
    criticals: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let cname = &cs.name;

    if let Some(state) = &cs.state {
        if let Some(waiting) = &state.waiting {
            if let Some(reason) = &waiting.reason {
                match reason.as_str() {
                    "CrashLoopBackOff"
                    | "ImagePullBackOff"
                    | "ErrImagePull"
                    | "CreateContainerError"
                    | "CreateContainerConfigError"
                    | "InvalidImageName" => {
                        criticals.push(format!("{pod_name}/{cname}: {reason}"));
                        return;
                    }
                    _ => {}
                }
            }
        }
    }

    if cs.restart_count > 0 {
        let finished_at = cs
            .last_state
            .as_ref()
            .and_then(|s| s.terminated.as_ref())
            .and_then(|t| t.finished_at.as_ref())
            .map(|t| t.0.as_second());

        if let Some(fin) = finished_at {
            let age = now_ts - fin;
            if (0..RECENT_RESTART_WINDOW_SECS).contains(&age) {
                let minutes = age / 60;
                let reason = cs
                    .last_state
                    .as_ref()
                    .and_then(|s| s.terminated.as_ref())
                    .and_then(|t| t.reason.clone())
                    .unwrap_or_else(|| "restarted".to_string());
                warnings.push(format!(
                    "{pod_name}/{cname}: {reason} {minutes}m ago (restarts={})",
                    cs.restart_count
                ));
            }
        }
    }
}
