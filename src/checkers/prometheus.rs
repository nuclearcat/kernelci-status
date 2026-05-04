use crate::checkers::{CheckContext, CheckResult, EndpointState};
use crate::db::endpoints::Endpoint;
use std::io::BufRead;

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
    let url = &endpoint.endpoint;

    let resp = ctx
        .http_client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to scrape metrics: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Metrics endpoint returned {}", resp.status()));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read body: {e}"))?;

    let selector = endpoint.selector.as_deref().unwrap_or("");
    if selector.is_empty() {
        return Ok(CheckResult {
            state: EndpointState::Ok,
            value: None,
            message: Some("Metrics endpoint reachable".to_string()),
        });
    }

    let (metric_name, labels) = parse_selector(selector)?;

    // prometheus-parse expects an iterator of Result<String, io::Error>
    let lines = body
        .as_bytes()
        .lines()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read lines: {e}"))?;
    let scrape = prometheus_parse::Scrape::parse(lines.iter().map(|s| Ok(s.clone())))
        .map_err(|e| format!("Failed to parse metrics: {e}"))?;

    for sample in &scrape.samples {
        if sample.metric != metric_name {
            continue;
        }

        let labels_match = labels
            .iter()
            .all(|(key, val)| sample.labels.get(key.as_str()).is_some_and(|v| v == val));

        if labels_match {
            let value = match &sample.value {
                prometheus_parse::Value::Counter(v)
                | prometheus_parse::Value::Gauge(v)
                | prometheus_parse::Value::Untyped(v) => v.to_string(),
                prometheus_parse::Value::Histogram(h) => {
                    format!("{}", h.iter().map(|b| b.count).sum::<f64>())
                }
                prometheus_parse::Value::Summary(s) => {
                    format!("{}", s.iter().map(|q| q.count).sum::<f64>())
                }
            };

            return Ok(CheckResult {
                state: EndpointState::Ok,
                value: Some(value.clone()),
                message: Some(format!("{metric_name} = {value}")),
            });
        }
    }

    Ok(CheckResult {
        state: EndpointState::NoData,
        value: None,
        message: Some(format!(
            "Metric {metric_name} not found with specified labels"
        )),
    })
}

fn parse_selector(s: &str) -> Result<(String, Vec<(String, String)>), String> {
    let s = s.trim();
    let s = s
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(s);

    let parts: Vec<&str> = s.splitn(2, ',').collect();
    let metric_name = parts.first().ok_or("Empty selector")?.trim().to_string();

    let mut labels = Vec::new();
    if parts.len() > 1 {
        for label_part in parts[1].split(',') {
            let label_part = label_part.trim();
            if let Some((key, val)) = label_part.split_once('=') {
                labels.push((key.trim().to_string(), val.trim().to_string()));
            }
        }
    }

    Ok((metric_name, labels))
}
