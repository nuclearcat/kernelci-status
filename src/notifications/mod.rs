// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

pub mod discord;
pub mod email;
pub mod telegram;
pub mod textfile;

use std::collections::HashMap;
use tokio::sync::mpsc;
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct NotificationEvent {
    pub endpoint_name: String,
    pub subname: Option<String>,
    pub old_state: String,
    pub new_state: String,
    pub message: Option<String>,
    pub value: Option<String>,
}

/// Classify a state transition: WARNING ↔ OK transitions are "warning",
/// everything else (anything involving CRITICAL, NO_DATA, or unknown states)
/// is "critical". `_MAINTENANCE` suffixes are ignored for classification.
pub fn transition_kind(old_state: &str, new_state: &str) -> &'static str {
    fn base(s: &str) -> &str {
        s.strip_suffix("_MAINTENANCE").unwrap_or(s)
    }
    let is_ok_warn = |s: &str| matches!(base(s), "OK" | "WARNING");
    if is_ok_warn(old_state) && is_ok_warn(new_state) {
        "warning"
    } else {
        "critical"
    }
}

/// Check whether a backend should receive a given kind of notification
/// ("critical", "warning" or "maintenance"). Missing keys default to true so
/// configs saved before these toggles existed keep sending everything.
pub fn backend_wants(config: &HashMap<String, String>, backend: &str, kind: &str) -> bool {
    config
        .get(&format!("{backend}_notify_{kind}"))
        .is_none_or(|v| v == "true")
}

pub async fn notification_worker(
    mut rx: mpsc::Receiver<NotificationEvent>,
    db: tokio_rusqlite::Connection,
    http_client: reqwest::Client,
) {
    info!("Notification worker started");
    while let Some(event) = rx.recv().await {
        let config = match load_notification_config(&db).await {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to load notification config: {e}");
                continue;
            }
        };

        let display_name = match &event.subname {
            Some(sub) => format!("{} ({})", event.endpoint_name, sub),
            None => event.endpoint_name.clone(),
        };

        let text = format!(
            "[{}] {} → {} | {}{}",
            match event.new_state.as_str() {
                "CRITICAL" => "CRITICAL",
                "WARNING" => "WARNING",
                "OK" => "OK",
                _ => "service",
            },
            display_name,
            event.new_state,
            event.message.as_deref().unwrap_or(""),
            event
                .value
                .as_ref()
                .map(|v| format!(" (value: {v})"))
                .unwrap_or_default(),
        );

        let kind = transition_kind(&event.old_state, &event.new_state);

        let mut tasks = Vec::new();

        if config.get("discord_enabled").is_some_and(|v| v == "true")
            && backend_wants(&config, "discord", kind)
        {
            if let Some(webhook_url) = config.get("discord_webhook_url") {
                let client = http_client.clone();
                let url = webhook_url.clone();
                let msg = text.clone();
                tasks.push(tokio::spawn(async move {
                    if let Err(e) = discord::send(&client, &url, &msg).await {
                        error!("Discord notification failed: {e}");
                    }
                }));
            }
        }

        if config.get("telegram_enabled").is_some_and(|v| v == "true")
            && backend_wants(&config, "telegram", kind)
        {
            let token = config
                .get("telegram_bot_token")
                .cloned()
                .unwrap_or_default();
            let chat_id = config.get("telegram_chat_id").cloned().unwrap_or_default();
            if !token.is_empty() && !chat_id.is_empty() {
                let client = http_client.clone();
                let msg = text.clone();
                tasks.push(tokio::spawn(async move {
                    if let Err(e) = telegram::send(&client, &token, &chat_id, &msg).await {
                        error!("Telegram notification failed: {e}");
                    }
                }));
            }
        }

        if config.get("email_enabled").is_some_and(|v| v == "true")
            && backend_wants(&config, "email", kind)
        {
            let cfg = config.clone();
            let ev = event.clone();
            tasks.push(tokio::spawn(async move {
                if let Err(e) = email::send_notification(&cfg, &ev).await {
                    error!("Email notification failed: {e}");
                }
            }));
        }

        if config.get("textfile_enabled").is_some_and(|v| v == "true")
            && backend_wants(&config, "textfile", kind)
        {
            if let Some(path) = config.get("textfile_path") {
                let path = path.clone();
                let msg = text.clone();
                tasks.push(tokio::spawn(async move {
                    if let Err(e) = textfile::append(&path, &msg).await {
                        error!("Text file notification failed: {e}");
                    }
                }));
            }
        }

        for task in tasks {
            let _ = task.await;
        }
    }
    info!("Notification worker stopped");
}

async fn load_notification_config(
    db: &tokio_rusqlite::Connection,
) -> Result<HashMap<String, String>, crate::error::DbError> {
    db.call(|conn| {
        let pairs = crate::db::config::get_all(conn)?;
        Ok(pairs.into_iter().collect())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_kind_classifies_states() {
        assert_eq!(transition_kind("OK", "WARNING"), "warning");
        assert_eq!(transition_kind("WARNING", "OK"), "warning");
        assert_eq!(transition_kind("WARNING_MAINTENANCE", "OK"), "warning");
        assert_eq!(transition_kind("OK", "CRITICAL"), "critical");
        assert_eq!(transition_kind("CRITICAL", "OK"), "critical");
        assert_eq!(transition_kind("CRITICAL_MAINTENANCE", "OK"), "critical");
        assert_eq!(transition_kind("NO_DATA", "OK"), "critical");
    }

    #[test]
    fn backend_wants_defaults_to_true() {
        let mut config = HashMap::new();
        assert!(backend_wants(&config, "discord", "critical"));
        config.insert("discord_notify_critical".to_string(), "false".to_string());
        assert!(!backend_wants(&config, "discord", "critical"));
        config.insert("discord_notify_critical".to_string(), "true".to_string());
        assert!(backend_wants(&config, "discord", "critical"));
    }
}
