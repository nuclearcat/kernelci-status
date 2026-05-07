// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

pub mod discord;
pub mod email;
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

        let mut tasks = Vec::new();

        if config.get("discord_enabled").is_some_and(|v| v == "true") {
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

        if config.get("email_enabled").is_some_and(|v| v == "true") {
            let cfg = config.clone();
            let ev = event.clone();
            tasks.push(tokio::spawn(async move {
                if let Err(e) = email::send_notification(&cfg, &ev).await {
                    error!("Email notification failed: {e}");
                }
            }));
        }

        if config.get("textfile_enabled").is_some_and(|v| v == "true") {
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
