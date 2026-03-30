use crate::checkers::{CheckContext, CheckResult, EndpointState};
use crate::db::endpoints::Endpoint;
use k8s_openapi::api::core::v1::Pod;
use kube::{Api, Client, Config};
use std::time::Duration;

const API_TIMEOUT: Duration = Duration::from_secs(15);

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
    // endpoint format: k8s://namespace/label-selector
    let url = &endpoint.endpoint;
    let path = url.strip_prefix("k8s://").unwrap_or(url);
    let (namespace, label_selector) = match path.split_once('/') {
        Some((ns, sel)) => (ns, Some(sel)),
        None => (path, None),
    };

    let config = tokio::time::timeout(API_TIMEOUT, Config::infer())
        .await
        .map_err(|_| "Kubeconfig load timed out (15s)".to_string())?
        .map_err(|e| format!("Failed to load kubeconfig: {e}"))?;
    let client = Client::try_from(config)
        .map_err(|e| format!("Failed to create k8s client: {e}"))?;

    let pods: Api<Pod> = Api::namespaced(client, namespace);
    let mut lp = kube::api::ListParams::default();
    if let Some(sel) = label_selector {
        lp = lp.labels(sel);
    }

    let pod_list = tokio::time::timeout(API_TIMEOUT, pods.list(&lp))
        .await
        .map_err(|_| "K8s API request timed out (15s)".to_string())?
        .map_err(|e| format!("Failed to list pods: {e}"))?;

    let mut warnings = Vec::new();
    let now = chrono::Utc::now();

    for pod in &pod_list.items {
        let pod_name = pod.metadata.name.as_deref().unwrap_or("unknown");

        if let Some(status) = &pod.status {
            if let Some(start_time) = status.start_time.as_ref() {
                let start_ts = start_time.0.as_second();
                let now_ts = now.timestamp();
                let uptime_minutes = (now_ts - start_ts) / 60;
                if uptime_minutes < 30 {
                    warnings.push(format!("{pod_name}: uptime {uptime_minutes}m"));
                }
            }
        }
    }

    let total_pods = pod_list.items.len();
    if !warnings.is_empty() {
        Ok(CheckResult {
            state: EndpointState::Warning,
            value: Some(total_pods.to_string()),
            message: Some(format!("Pods with low uptime: {}", warnings.join(", "))),
        })
    } else {
        Ok(CheckResult {
            state: EndpointState::Ok,
            value: Some(total_pods.to_string()),
            message: Some(format!("{total_pods} pods running, all uptime >30m")),
        })
    }
}
