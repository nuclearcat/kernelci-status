// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use serde_json::json;

pub async fn send(
    client: &reqwest::Client,
    webhook_url: &str,
    message: &str,
) -> Result<(), String> {
    let body = json!({
        "embeds": [{
            "description": message,
            "color": 16711680
        }]
    });

    let resp = client
        .post(webhook_url)
        .json(&body)
        .send()
        .await
        .map_err(|_| "Discord webhook request failed".to_string())?;

    if !resp.status().is_success() {
        return Err(format!("Discord webhook returned {}", resp.status()));
    }
    Ok(())
}
