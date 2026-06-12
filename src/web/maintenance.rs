// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect};
use axum_extra::extract::Form;
use serde::Deserialize;
use std::collections::{BTreeSet, HashSet};

use tracing::error;

use crate::auth::AuthUser;
use crate::db::maintenance::NewMaintenanceWindow;
use crate::error::AppError;
use crate::state::AppState;

/// Maintainers may only manage maintenance for endpoints within their team scope.
/// Returns `None` for admins (unrestricted), or `Some(allowed endpoint names)`
/// for a maintainer (the union across their teams — empty if they're on no team).
async fn scope_for(state: &AppState, user: &AuthUser) -> Result<Option<HashSet<String>>, AppError> {
    if user.role == "admin" {
        return Ok(None);
    }
    let uid = user.user_id;
    let db = state.db.clone();
    let names = db
        .call(move |conn| crate::db::teams::allowed_endpoint_names_for_user(conn, uid))
        .await?;
    Ok(Some(names))
}

/// Reject if a maintainer submits endpoint names outside their scope, or none.
fn check_submitted_names(
    scope: &Option<HashSet<String>>,
    names: &[String],
) -> Result<(), AppError> {
    if let Some(allowed) = scope {
        if names.is_empty() {
            return Err(AppError::BadRequest(
                "Select at least one endpoint from your team".to_string(),
            ));
        }
        if let Some(bad) = names.iter().find(|n| !allowed.contains(*n)) {
            return Err(AppError::BadRequest(format!(
                "Not authorized to schedule maintenance for '{bad}'"
            )));
        }
    }
    Ok(())
}

/// Reject if a maintainer tries to act on an existing window whose endpoints
/// aren't fully within their scope. No-op for admins.
async fn check_window_ownership(
    state: &AppState,
    scope: &Option<HashSet<String>>,
    window_id: i64,
) -> Result<(), AppError> {
    let Some(allowed) = scope else {
        return Ok(());
    };
    let db = state.db.clone();
    let names = db
        .call(move |conn| crate::db::maintenance::get_endpoint_names(conn, window_id))
        .await?;
    if names.is_empty() || !names.iter().all(|n| allowed.contains(n)) {
        return Err(AppError::BadRequest(
            "Not authorized to manage this maintenance window".to_string(),
        ));
    }
    Ok(())
}

/// View model for a maintenance window (with resolved endpoint names and truncated times).
struct WindowView {
    id: i64,
    name: String,
    start_time: String,
    end_time: String,
    is_active: bool,
    is_past: bool,
    endpoint_names: Vec<String>,
    is_deploy: bool,
    changelog: String,
}

#[derive(Template)]
#[template(path = "maintenance.html")]
struct MaintenanceTemplate {
    username: String,
    is_admin: bool,
    windows: Vec<WindowView>,
    unique_names: Vec<String>,
    now: String,
}

pub async fn maintenance_page(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let (windows, endpoints) = db
        .call(|conn| {
            let w = crate::db::maintenance::list_all(conn)?;
            let e = crate::db::endpoints::list_all(conn)?;
            Ok((w, e))
        })
        .await?;

    // Maintainers are restricted to the endpoints in their team scope.
    let scope = scope_for(&state, &user).await?;
    let is_admin = scope.is_none();

    // Unique endpoint names (sorted), restricted to scope for maintainers.
    let unique_names: Vec<String> = {
        let set: BTreeSet<String> = endpoints
            .iter()
            .map(|e| e.name.clone())
            .filter(|n| scope.as_ref().is_none_or(|allowed| allowed.contains(n)))
            .collect();
        set.into_iter().collect()
    };

    let now_full = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // Build view models with resolved endpoint names and truncated times
    let mut windows: Vec<WindowView> = windows
        .into_iter()
        .map(|w| {
            let names: BTreeSet<String> = w
                .endpoint_ids
                .iter()
                .filter_map(|eid| endpoints.iter().find(|e| e.id == *eid))
                .map(|e| e.name.clone())
                .collect();
            WindowView {
                id: w.id,
                name: w.name,
                is_active: w.start_time.as_str() <= now_full.as_str()
                    && w.end_time.as_str() > now_full.as_str(),
                is_past: w.end_time.as_str() <= now_full.as_str(),
                start_time: truncate_seconds(&w.start_time),
                end_time: truncate_seconds(&w.end_time),
                endpoint_names: names.into_iter().collect(),
                is_deploy: w.is_deploy,
                changelog: w.changelog.unwrap_or_default(),
            }
        })
        .collect();

    // Maintainers only see windows whose endpoints are fully within their scope.
    if let Some(allowed) = &scope {
        windows.retain(|w| {
            !w.endpoint_names.is_empty() && w.endpoint_names.iter().all(|n| allowed.contains(n))
        });
    }

    let now = truncate_seconds(&now_full);

    Ok(Html(
        MaintenanceTemplate {
            username: user.username,
            is_admin,
            windows,
            unique_names,
            now,
        }
        .render()
        .unwrap_or_default(),
    ))
}

/// Truncate "YYYY-MM-DD HH:MM:SS" to "YYYY-MM-DD HH:MM".
fn truncate_seconds(s: &str) -> String {
    if s.len() >= 16 {
        s[..16].to_string()
    } else {
        s.to_string()
    }
}

#[derive(Deserialize)]
pub struct MaintenanceForm {
    pub name: String,
    pub start_time: String,
    pub end_time: String,
    #[serde(default)]
    pub endpoint_names: Vec<String>,
    #[serde(default)]
    pub is_deploy: Option<String>,
    #[serde(default)]
    pub changelog: Option<String>,
}

fn normalize_time(t: &str) -> String {
    let s = t.replace('T', " ");
    if s.len() == 16 {
        format!("{}:00", s)
    } else {
        s
    }
}

fn validate_not_in_past(form: &MaintenanceForm) -> Result<(), AppError> {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M").to_string();
    if form.start_time < now {
        return Err(AppError::BadRequest(
            "Start time cannot be in the past".to_string(),
        ));
    }
    if form.end_time < now {
        return Err(AppError::BadRequest(
            "End time cannot be in the past".to_string(),
        ));
    }
    if form.end_time <= form.start_time {
        return Err(AppError::BadRequest(
            "End time must be after start time".to_string(),
        ));
    }
    Ok(())
}

pub async fn add_maintenance(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<MaintenanceForm>,
) -> Result<impl IntoResponse, AppError> {
    let scope = scope_for(&state, &user).await?;
    check_submitted_names(&scope, &form.endpoint_names)?;
    validate_not_in_past(&form)?;
    let start = normalize_time(&form.start_time);
    let end = normalize_time(&form.end_time);
    let mw_name = form.name.clone();
    let names = form.endpoint_names.clone();
    let is_deploy = form.is_deploy.as_deref() == Some("on");
    let changelog = if is_deploy {
        form.changelog.clone().filter(|c| !c.trim().is_empty())
    } else {
        None
    };

    let db = state.db.clone();
    db.call(move |conn| {
        let endpoint_ids = crate::db::endpoints::get_ids_by_names(conn, &names)?;
        let mw = NewMaintenanceWindow {
            name: mw_name,
            start_time: start,
            end_time: end,
            endpoint_ids,
            is_deploy,
            changelog,
        };
        crate::db::maintenance::insert(conn, &mw)
    })
    .await?;
    Ok(Redirect::to("/admin/maintenance"))
}

pub async fn edit_maintenance(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<MaintenanceForm>,
) -> Result<impl IntoResponse, AppError> {
    let scope = scope_for(&state, &user).await?;
    // Must own the window as it stands AND only target endpoints within scope.
    check_window_ownership(&state, &scope, id).await?;
    check_submitted_names(&scope, &form.endpoint_names)?;
    validate_not_in_past(&form)?;
    let start = normalize_time(&form.start_time);
    let end = normalize_time(&form.end_time);
    let mw_name = form.name.clone();
    let names = form.endpoint_names.clone();
    let is_deploy = form.is_deploy.as_deref() == Some("on");
    let changelog = if is_deploy {
        form.changelog.clone().filter(|c| !c.trim().is_empty())
    } else {
        None
    };

    let db = state.db.clone();
    db.call(move |conn| {
        let endpoint_ids = crate::db::endpoints::get_ids_by_names(conn, &names)?;
        let mw = NewMaintenanceWindow {
            name: mw_name,
            start_time: start,
            end_time: end,
            endpoint_ids,
            is_deploy,
            changelog,
        };
        crate::db::maintenance::update(conn, id, &mw)
    })
    .await?;
    Ok(Redirect::to("/admin/maintenance"))
}

pub async fn delete_maintenance(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let scope = scope_for(&state, &user).await?;
    check_window_ownership(&state, &scope, id).await?;
    let db = state.db.clone();
    db.call(move |conn| crate::db::maintenance::delete(conn, id))
        .await?;
    Ok(Redirect::to("/admin/maintenance"))
}

pub async fn close_maintenance(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let scope = scope_for(&state, &user).await?;
    check_window_ownership(&state, &scope, id).await?;
    let ended_at = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let db = state.db.clone();
    let closed = db
        .call(move |conn| crate::db::maintenance::close_early(conn, id, &ended_at))
        .await?;
    if !closed {
        return Err(AppError::BadRequest(
            "Maintenance window is not currently active".to_string(),
        ));
    }
    Ok(Redirect::to("/admin/maintenance"))
}

/// Check for maintenance windows starting within 1 hour and send reminders
/// through all enabled notification backends.
/// Called from the scheduler after each check cycle.
pub async fn check_maintenance_reminders(state: &AppState) {
    let db = state.db.clone();

    let windows = match db
        .call(|conn| {
            let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let windows = crate::db::maintenance::get_needing_reminder(conn, &now)?;
            let endpoints = crate::db::endpoints::list_all(conn)?;

            // Resolve endpoint IDs to names and collect window data
            let mut result = Vec::new();
            for w in &windows {
                let names: Vec<String> = w
                    .endpoint_ids
                    .iter()
                    .filter_map(|eid| endpoints.iter().find(|e| e.id == *eid))
                    .map(|e| e.name.clone())
                    .collect();
                // De-duplicate names
                let mut unique: Vec<String> = names;
                unique.sort();
                unique.dedup();
                result.push((
                    w.id,
                    w.name.clone(),
                    w.start_time.clone(),
                    w.end_time.clone(),
                    unique,
                    w.is_deploy,
                    w.changelog.clone(),
                ));
            }
            Ok::<_, rusqlite::Error>(result)
        })
        .await
    {
        Ok(w) => w,
        Err(e) => {
            error!("Failed to check maintenance reminders: {e}");
            return;
        }
    };

    if windows.is_empty() {
        return;
    }

    // Load notification config
    let config: std::collections::HashMap<String, String> = match db
        .call(|conn| {
            let pairs = crate::db::config::get_all(conn)?;
            Ok::<_, rusqlite::Error>(pairs.into_iter().collect())
        })
        .await
    {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load config for maintenance reminders: {e}");
            return;
        }
    };

    // Which backends should receive maintenance reminders?
    let wants = |backend: &str| {
        config
            .get(&format!("{backend}_enabled"))
            .is_some_and(|v| v == "true")
            && crate::notifications::backend_wants(&config, backend, "maintenance")
    };
    let (email_on, discord_on, telegram_on, textfile_on) = (
        wants("email"),
        wants("discord"),
        wants("telegram"),
        wants("textfile"),
    );
    if !email_on && !discord_on && !telegram_on && !textfile_on {
        return;
    }

    for (id, name, start_time, end_time, endpoint_names, is_deploy, changelog) in &windows {
        let mut sent = false;

        if email_on {
            let data = crate::notifications::email::MaintenanceReminderData {
                window_name: name.clone(),
                start_time: start_time.clone(),
                end_time: end_time.clone(),
                endpoint_names: endpoint_names.clone(),
                is_deploy: *is_deploy,
                changelog: changelog.clone(),
            };
            match crate::notifications::email::send_maintenance_reminder(&config, &data).await {
                Ok(()) => sent = true,
                Err(e) => error!("Failed to send maintenance reminder for '{}': {e}", name),
            }
        }

        let deploy_label = if *is_deploy { " (Deploy)" } else { "" };
        let affected = if endpoint_names.is_empty() {
            "None specified".to_string()
        } else {
            endpoint_names.join(", ")
        };
        let text = format!(
            "[maintenance] {name}{deploy_label} — starting in less than 1 hour | \
             {start_time} – {end_time} UTC | Affected: {affected}"
        );

        if discord_on {
            if let Some(url) = config.get("discord_webhook_url").filter(|u| !u.is_empty()) {
                match crate::notifications::discord::send(&state.http_client, url, &text).await {
                    Ok(()) => sent = true,
                    Err(e) => error!("Discord maintenance reminder failed for '{}': {e}", name),
                }
            }
        }

        if telegram_on {
            let token = config
                .get("telegram_bot_token")
                .cloned()
                .unwrap_or_default();
            let chat_id = config.get("telegram_chat_id").cloned().unwrap_or_default();
            if !token.is_empty() && !chat_id.is_empty() {
                match crate::notifications::telegram::send(
                    &state.http_client,
                    &token,
                    &chat_id,
                    &text,
                )
                .await
                {
                    Ok(()) => sent = true,
                    Err(e) => error!("Telegram maintenance reminder failed for '{}': {e}", name),
                }
            }
        }

        if textfile_on {
            if let Some(path) = config.get("textfile_path").filter(|p| !p.is_empty()) {
                match crate::notifications::textfile::append(path, &text).await {
                    Ok(()) => sent = true,
                    Err(e) => error!("Text file maintenance reminder failed for '{}': {e}", name),
                }
            }
        }

        // Mark as sent if at least one backend delivered; otherwise retry next cycle
        if !sent {
            continue;
        }
        let wid = *id;
        if let Err(e) = db
            .call(move |conn| crate::db::maintenance::mark_reminder_sent(conn, wid))
            .await
        {
            error!("Failed to mark reminder sent for window {id}: {e}");
        }
    }
}
