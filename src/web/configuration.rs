use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::Form;
use serde::Deserialize;
use std::collections::HashMap;

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

pub struct AppConfigView {
    pub check_interval: String,
    pub check_retries: String,
    pub warning_retries: String,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_ssl: bool,
    pub smtp_tls: bool,
    pub email_from: String,
    pub email_from_name: String,
    pub base_url: String,
    pub incident_escalation_minutes: String,
}

impl AppConfigView {
    fn from_map(m: &HashMap<String, String>) -> Self {
        let g = |k: &str, def: &str| -> String {
            m.get(k).cloned().unwrap_or_else(|| def.to_string())
        };
        Self {
            check_interval: g("check_interval", "5"),
            check_retries: g("check_retries", "3"),
            warning_retries: g("warning_retries", "3"),
            smtp_host: g("smtp_host", ""),
            smtp_port: g("smtp_port", "587"),
            smtp_username: g("smtp_username", ""),
            smtp_password: g("smtp_password", ""),
            smtp_ssl: m.get("smtp_ssl").is_some_and(|v| v == "true"),
            smtp_tls: m.get("smtp_tls").is_some_and(|v| v == "true"),
            email_from: g("email_from", ""),
            email_from_name: g("email_from_name", "KernelCI Status"),
            base_url: g("base_url", ""),
            incident_escalation_minutes: g("incident_escalation_minutes", "30"),
        }
    }
}

#[derive(Template)]
#[template(path = "configuration.html")]
struct ConfigurationTemplate {
    username: String,
    c: AppConfigView,
    error: String,
    success: String,
}

pub async fn configuration_page(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let config = load_config(&state).await?;

    Ok(Html(
        ConfigurationTemplate {
            username: user.username,
            c: AppConfigView::from_map(&config),
            error: String::new(),
            success: String::new(),
        }
        .render()
        .unwrap_or_default(),
    ))
}

#[derive(Deserialize)]
pub struct ConfigurationForm {
    pub check_interval: Option<String>,
    pub check_retries: Option<String>,
    pub warning_retries: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<String>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_ssl: Option<String>,
    pub smtp_tls: Option<String>,
    pub email_from: Option<String>,
    pub email_from_name: Option<String>,
    pub base_url: Option<String>,
    pub incident_escalation_minutes: Option<String>,
}

pub async fn save_configuration(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<ConfigurationForm>,
) -> Result<impl IntoResponse, AppError> {
    // Validate scheduler settings
    let interval_str = form.check_interval.clone().unwrap_or_else(|| "5".to_string());
    if let Ok(v) = interval_str.parse::<u32>() {
        if v < 1 || v > 1440 {
            let config = load_config(&state).await?;
            return Ok(Html(
                ConfigurationTemplate {
                    username: user.username,
                    c: AppConfigView::from_map(&config),
                    error: "Check interval must be between 1 and 1440 minutes.".to_string(),
                    success: String::new(),
                }
                .render()
                .unwrap_or_default(),
            ));
        }
    } else if !interval_str.is_empty() {
        let config = load_config(&state).await?;
        return Ok(Html(
            ConfigurationTemplate {
                username: user.username,
                c: AppConfigView::from_map(&config),
                error: "Check interval must be a number.".to_string(),
                success: String::new(),
            }
            .render()
            .unwrap_or_default(),
        ));
    }

    let retries_str = form.check_retries.clone().unwrap_or_else(|| "3".to_string());
    if let Ok(v) = retries_str.parse::<u32>() {
        if v > 10 {
            let config = load_config(&state).await?;
            return Ok(Html(
                ConfigurationTemplate {
                    username: user.username,
                    c: AppConfigView::from_map(&config),
                    error: "Retries must be between 0 and 10.".to_string(),
                    success: String::new(),
                }
                .render()
                .unwrap_or_default(),
            ));
        }
    } else if !retries_str.is_empty() {
        let config = load_config(&state).await?;
        return Ok(Html(
            ConfigurationTemplate {
                username: user.username,
                c: AppConfigView::from_map(&config),
                error: "Retries must be a number.".to_string(),
                success: String::new(),
            }
            .render()
            .unwrap_or_default(),
        ));
    }

    let warning_retries_str = form.warning_retries.clone().unwrap_or_else(|| "3".to_string());
    if let Ok(v) = warning_retries_str.parse::<u32>() {
        if v > 10 {
            let config = load_config(&state).await?;
            return Ok(Html(
                ConfigurationTemplate {
                    username: user.username,
                    c: AppConfigView::from_map(&config),
                    error: "Warning retries must be between 0 and 10.".to_string(),
                    success: String::new(),
                }
                .render()
                .unwrap_or_default(),
            ));
        }
    } else if !warning_retries_str.is_empty() {
        let config = load_config(&state).await?;
        return Ok(Html(
            ConfigurationTemplate {
                username: user.username,
                c: AppConfigView::from_map(&config),
                error: "Warning retries must be a number.".to_string(),
                success: String::new(),
            }
            .render()
            .unwrap_or_default(),
        ));
    }

    // Validate from email if provided
    let from_email = form.email_from.as_deref().unwrap_or("").trim().to_string();
    if !from_email.is_empty() && !is_valid_email(&from_email) {
        let config = load_config(&state).await?;
        return Ok(Html(
            ConfigurationTemplate {
                username: user.username,
                c: AppConfigView::from_map(&config),
                error: format!("Invalid From email address: {from_email}"),
                success: String::new(),
            }
            .render()
            .unwrap_or_default(),
        ));
    }

    // Validate port
    let port_str = form.smtp_port.clone().unwrap_or_else(|| "587".to_string());
    if !port_str.is_empty() {
        if port_str.parse::<u16>().is_err() {
            let config = load_config(&state).await?;
            return Ok(Html(
                ConfigurationTemplate {
                    username: user.username,
                    c: AppConfigView::from_map(&config),
                    error: "SMTP Port must be a number between 1 and 65535.".to_string(),
                    success: String::new(),
                }
                .render()
                .unwrap_or_default(),
            ));
        }
    }

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

        set("check_interval", &interval_str)?;
        set("check_retries", &retries_str)?;
        set("warning_retries", &warning_retries_str)?;
        set("smtp_host", form.smtp_host.as_deref().unwrap_or(""))?;
        set("smtp_port", &port_str)?;
        set("smtp_username", form.smtp_username.as_deref().unwrap_or(""))?;
        // Only update password if a new one was submitted (field is never pre-filled)
        let new_pw = form.smtp_password.as_deref().unwrap_or("");
        if !new_pw.is_empty() {
            set("smtp_password", new_pw)?;
        }
        set_toggle("smtp_ssl", &form.smtp_ssl)?;
        set_toggle("smtp_tls", &form.smtp_tls)?;
        set("email_from", &from_email)?;
        set("email_from_name", form.email_from_name.as_deref().unwrap_or("KernelCI Status"))?;
        // Strip trailing slash from base_url
        let base = form.base_url.as_deref().unwrap_or("").trim_end_matches('/');
        set("base_url", base)?;
        set("incident_escalation_minutes", form.incident_escalation_minutes.as_deref().unwrap_or("30"))?;
        Ok(())
    })
    .await?;

    // Update config cache
    let new_config = load_config_from_db(&state).await?;
    *state.config_cache.write().await = new_config;

    Ok(Html(
        ConfigurationTemplate {
            username: user.username,
            c: AppConfigView::from_map(&load_config(&state).await?),
            error: String::new(),
            success: "Configuration saved.".to_string(),
        }
        .render()
        .unwrap_or_default(),
    ))
}

pub async fn test_email(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let config = load_config(&state).await?;
    let cfg_view = AppConfigView::from_map(&config);

    // Validate SMTP is configured
    if cfg_view.smtp_host.is_empty() {
        return Ok(Html(
            ConfigurationTemplate {
                username: user.username,
                c: cfg_view,
                error: "SMTP Server is not configured. Save SMTP settings first.".to_string(),
                success: String::new(),
            }
            .render()
            .unwrap_or_default(),
        ));
    }

    if cfg_view.email_from.is_empty() {
        return Ok(Html(
            ConfigurationTemplate {
                username: user.username,
                c: cfg_view,
                error: "From Email is not configured. Save SMTP settings first.".to_string(),
                success: String::new(),
            }
            .render()
            .unwrap_or_default(),
        ));
    }

    let email_to = config.get("email_to").cloned().unwrap_or_default();
    if email_to.is_empty() {
        return Ok(Html(
            ConfigurationTemplate {
                username: user.username,
                c: cfg_view,
                error: "No recipient email addresses configured. Add them in the Notifications page first.".to_string(),
                success: String::new(),
            }
            .render()
            .unwrap_or_default(),
        ));
    }

    // Send test email
    match crate::notifications::email::send_test(&config).await {
        Ok(()) => Ok(Html(
            ConfigurationTemplate {
                username: user.username,
                c: AppConfigView::from_map(&config),
                error: String::new(),
                success: format!("Test email sent successfully to: {email_to}"),
            }
            .render()
            .unwrap_or_default(),
        )),
        Err(e) => Ok(Html(
            ConfigurationTemplate {
                username: user.username,
                c: AppConfigView::from_map(&config),
                error: format!("Failed to send test email: {e}"),
                success: String::new(),
            }
            .render()
            .unwrap_or_default(),
        )),
    }
}

fn is_valid_email(email: &str) -> bool {
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    if parts.len() != 2 {
        return false;
    }
    let local = parts[0];
    let domain = parts[1];
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    if !domain.contains('.') {
        return false;
    }
    !email.contains(' ')
}

async fn load_config(state: &AppState) -> Result<HashMap<String, String>, AppError> {
    load_config_from_db(state).await.map_err(|e| e.into())
}

async fn load_config_from_db(
    state: &AppState,
) -> Result<HashMap<String, String>, crate::error::DbError> {
    let db = state.db.clone();
    db.call(|conn| {
        let pairs = crate::db::config::get_all(conn)?;
        Ok(pairs.into_iter().collect())
    })
    .await
}
