use askama::Template;
use axum::Form;
use axum::extract::{Multipart, State};
use axum::http::header;
use axum::response::{Html, IntoResponse};
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
        let g =
            |k: &str, def: &str| -> String { m.get(k).cloned().unwrap_or_else(|| def.to_string()) };
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
    let interval_str = form
        .check_interval
        .clone()
        .unwrap_or_else(|| "5".to_string());
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

    let retries_str = form
        .check_retries
        .clone()
        .unwrap_or_else(|| "3".to_string());
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

    let warning_retries_str = form
        .warning_retries
        .clone()
        .unwrap_or_else(|| "3".to_string());
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
        set(
            "email_from_name",
            form.email_from_name.as_deref().unwrap_or("KernelCI Status"),
        )?;
        // Strip trailing slash from base_url
        let base = form.base_url.as_deref().unwrap_or("").trim_end_matches('/');
        set("base_url", base)?;
        set(
            "incident_escalation_minutes",
            form.incident_escalation_minutes.as_deref().unwrap_or("30"),
        )?;
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

pub async fn download_backup(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let tmp_path = std::env::temp_dir().join(format!(
        "kernelci-status-backup-{}-{}.db",
        ts,
        std::process::id()
    ));
    let tmp_path_sql = tmp_path.to_string_lossy().to_string();

    let db = state.db.clone();
    db.call(move |conn| -> rusqlite::Result<()> {
        // Use a literal path — VACUUM INTO does not accept parameter binding.
        // We escape single quotes to keep the statement safe.
        let escaped = tmp_path_sql.replace('\'', "''");
        conn.execute_batch(&format!("VACUUM INTO '{escaped}'"))?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("Failed to create backup: {e}")))?;

    let bytes = tokio::fs::read(&tmp_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to read backup file: {e}")))?;
    let _ = tokio::fs::remove_file(&tmp_path).await;

    let filename = format!("kernelci-status-{ts}.db");
    let headers = [
        (header::CONTENT_TYPE, "application/vnd.sqlite3".to_string()),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        ),
    ];
    Ok((headers, bytes))
}

pub async fn restore_backup(
    State(state): State<AppState>,
    user: AuthUser,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let mut uploaded: Option<Vec<u8>> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Invalid upload: {e}")))?
    {
        if field.name() == Some("backup") {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| AppError::BadRequest(format!("Failed to read upload: {e}")))?;
            uploaded = Some(bytes.to_vec());
            break;
        }
    }

    let bytes = match uploaded {
        Some(b) if !b.is_empty() => b,
        _ => return render_with_error(&state, &user.username, "No backup file uploaded.").await,
    };

    // SQLite files start with the magic header "SQLite format 3\0"
    if bytes.len() < 16 || &bytes[..16] != b"SQLite format 3\0" {
        return render_with_error(
            &state,
            &user.username,
            "Uploaded file is not a valid SQLite database.",
        )
        .await;
    }

    let tmp_path = std::env::temp_dir().join(format!(
        "kernelci-status-restore-{}-{}.db",
        chrono::Utc::now().timestamp_millis(),
        std::process::id()
    ));
    tokio::fs::write(&tmp_path, &bytes)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write temp file: {e}")))?;

    // Validate schema by opening read-only and checking for expected core tables.
    let tmp_path_val = tmp_path.clone();
    let validation = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = rusqlite::Connection::open_with_flags(
            &tmp_path_val,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| format!("Cannot open uploaded file as SQLite: {e}"))?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='table' AND name IN ('endpoints','config','users')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| format!("Cannot inspect uploaded database: {e}"))?;
        if count < 3 {
            return Err(
                "Uploaded file is not a kernelci-status backup (missing required tables)."
                    .to_string(),
            );
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("Validation task failed: {e}")))?;

    if let Err(msg) = validation {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return render_with_error(&state, &user.username, &msg).await;
    }

    // Restore into the live connection. This atomically copies every page from
    // the source file into the destination database, so all other handlers
    // sharing this connection immediately see the restored state.
    let tmp_path_restore = tmp_path.clone();
    let restore_result = state
        .db
        .call(move |conn| -> rusqlite::Result<()> {
            let progress: Option<fn(rusqlite::backup::Progress)> = None;
            conn.restore("main", &tmp_path_restore, progress)?;
            Ok(())
        })
        .await;

    let _ = tokio::fs::remove_file(&tmp_path).await;

    if let Err(e) = restore_result {
        return Err(AppError::Internal(format!("Failed to restore backup: {e}")));
    }

    // Refresh the in-memory config cache from the newly restored database.
    let new_config = load_config_from_db(&state).await?;
    *state.config_cache.write().await = new_config;

    let config = load_config(&state).await?;
    Ok(Html(
        ConfigurationTemplate {
            username: user.username,
            c: AppConfigView::from_map(&config),
            error: String::new(),
            success: "Backup restored successfully.".to_string(),
        }
        .render()
        .unwrap_or_default(),
    )
    .into_response())
}

async fn render_with_error(
    state: &AppState,
    username: &str,
    msg: &str,
) -> Result<axum::response::Response, AppError> {
    let config = load_config(state).await?;
    Ok(Html(
        ConfigurationTemplate {
            username: username.to_string(),
            c: AppConfigView::from_map(&config),
            error: msg.to_string(),
            success: String::new(),
        }
        .render()
        .unwrap_or_default(),
    )
    .into_response())
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
