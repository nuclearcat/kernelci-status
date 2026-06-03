// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use askama::Template;
use axum::Form;
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use serde::Deserialize;
use std::collections::HashMap;

use crate::auth::AdminUser;
use crate::error::AppError;
use crate::state::AppState;
use crate::web::common::{is_valid_email, load_config, load_config_from_db};

pub struct NotifConfig {
    pub discord_enabled: bool,
    pub discord_webhook_url: String,
    pub telegram_enabled: bool,
    pub telegram_bot_token: String,
    pub telegram_chat_id: String,
    pub email_enabled: bool,
    pub email_to: String,
    pub email_to_warnings: String,
    pub textfile_enabled: bool,
    pub textfile_path: String,
}

impl NotifConfig {
    fn from_map(m: &HashMap<String, String>) -> Self {
        let g =
            |k: &str, def: &str| -> String { m.get(k).cloned().unwrap_or_else(|| def.to_string()) };
        Self {
            discord_enabled: m.get("discord_enabled").is_some_and(|v| v == "true"),
            discord_webhook_url: g("discord_webhook_url", ""),
            telegram_enabled: m.get("telegram_enabled").is_some_and(|v| v == "true"),
            telegram_bot_token: g("telegram_bot_token", ""),
            telegram_chat_id: g("telegram_chat_id", ""),
            email_enabled: m.get("email_enabled").is_some_and(|v| v == "true"),
            email_to: g("email_to", ""),
            email_to_warnings: g("email_to_warnings", ""),
            textfile_enabled: m.get("textfile_enabled").is_some_and(|v| v == "true"),
            textfile_path: g("textfile_path", ""),
        }
    }
}

#[derive(Template)]
#[template(path = "notifications.html")]
struct NotificationsTemplate {
    username: String,
    c: NotifConfig,
    error: String,
    success: String,
}

pub async fn notifications_page(
    State(state): State<AppState>,
    user: AdminUser,
) -> Result<impl IntoResponse, AppError> {
    let config = load_config(&state).await?;

    Ok(Html(
        NotificationsTemplate {
            username: user.username,
            c: NotifConfig::from_map(&config),
            error: String::new(),
            success: String::new(),
        }
        .render()
        .unwrap_or_default(),
    ))
}

#[derive(Deserialize)]
pub struct NotificationForm {
    pub discord_enabled: Option<String>,
    pub discord_webhook_url: Option<String>,
    pub telegram_enabled: Option<String>,
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
    pub email_enabled: Option<String>,
    pub email_to: Option<String>,
    pub email_to_warnings: Option<String>,
    pub textfile_enabled: Option<String>,
    pub textfile_path: Option<String>,
}

/// Validate a comma-separated list of email addresses.
/// Returns Ok with the cleaned string, or Err with a message listing invalid addresses.
fn validate_emails(raw: &str) -> Result<String, String> {
    if raw.trim().is_empty() {
        return Ok(String::new());
    }
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for part in raw.split(',') {
        let addr = part.trim();
        if addr.is_empty() {
            continue;
        }
        if is_valid_email(addr) {
            valid.push(addr.to_string());
        } else {
            invalid.push(addr.to_string());
        }
    }
    if !invalid.is_empty() {
        return Err(format!("Invalid email address(es): {}", invalid.join(", ")));
    }
    Ok(valid.join(", "))
}

pub async fn save_notifications(
    State(state): State<AppState>,
    user: AdminUser,
    Form(form): Form<NotificationForm>,
) -> Result<impl IntoResponse, AppError> {
    let email_to_raw = form.email_to.clone().unwrap_or_default();
    let email_to_warnings_raw = form.email_to_warnings.clone().unwrap_or_default();

    // Validate emails if email is being enabled
    let validation = validate_emails(&email_to_raw)
        .and_then(|main| validate_emails(&email_to_warnings_raw).map(|warn| (main, warn)));

    let (cleaned_emails, cleaned_warning_emails) = match validation {
        Ok(pair) => pair,
        Err(err) => {
            let config = load_config(&state).await?;
            let mut c = NotifConfig::from_map(&config);
            // Show what the user typed so they can fix it
            c.email_to = email_to_raw;
            c.email_to_warnings = email_to_warnings_raw;
            return Ok(Html(
                NotificationsTemplate {
                    username: user.username,
                    c,
                    error: err,
                    success: String::new(),
                }
                .render()
                .unwrap_or_default(),
            ));
        }
    };

    let db = state.db.clone();
    db.call(move |conn| {
        let set = |key: &str, val: &str| -> rusqlite::Result<()> {
            crate::db::config::set(conn, key, val)
        };
        let set_toggle = |key: &str, val: &Option<String>| -> rusqlite::Result<()> {
            let v = if val.as_deref() == Some("on") {
                "true"
            } else {
                "false"
            };
            crate::db::config::set(conn, key, v)
        };

        set_toggle("discord_enabled", &form.discord_enabled)?;
        set(
            "discord_webhook_url",
            form.discord_webhook_url.as_deref().unwrap_or(""),
        )?;
        set_toggle("telegram_enabled", &form.telegram_enabled)?;
        // Only update the bot token if a new one was submitted (field is never
        // pre-filled, mirroring the SMTP password handling).
        let new_token = form.telegram_bot_token.as_deref().unwrap_or("").trim();
        if !new_token.is_empty() {
            set("telegram_bot_token", new_token)?;
        }
        set(
            "telegram_chat_id",
            form.telegram_chat_id.as_deref().unwrap_or("").trim(),
        )?;
        set_toggle("email_enabled", &form.email_enabled)?;
        set("email_to", &cleaned_emails)?;
        set("email_to_warnings", &cleaned_warning_emails)?;
        set_toggle("textfile_enabled", &form.textfile_enabled)?;
        set("textfile_path", form.textfile_path.as_deref().unwrap_or(""))?;
        Ok(())
    })
    .await?;

    // Update config cache
    let new_config = load_config_from_db(&state).await?;
    *state.config_cache.write().await = new_config;

    Ok(Html(
        NotificationsTemplate {
            username: user.username,
            c: NotifConfig::from_map(&load_config(&state).await?),
            error: String::new(),
            success: "Notification settings saved.".to_string(),
        }
        .render()
        .unwrap_or_default(),
    ))
}
