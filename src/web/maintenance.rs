use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect};
use axum_extra::extract::Form;
use serde::Deserialize;
use std::collections::BTreeSet;

use tracing::error;

use crate::auth::AuthUser;
use crate::db::maintenance::NewMaintenanceWindow;
use crate::error::AppError;
use crate::state::AppState;

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

    // Unique endpoint names (sorted)
    let unique_names: Vec<String> = {
        let set: BTreeSet<String> = endpoints.iter().map(|e| e.name.clone()).collect();
        set.into_iter().collect()
    };

    let now_full = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // Build view models with resolved endpoint names and truncated times
    let windows: Vec<WindowView> = windows
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

    let now = truncate_seconds(&now_full);

    Ok(Html(
        MaintenanceTemplate {
            username: user.username,
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
    _user: AuthUser,
    Form(form): Form<MaintenanceForm>,
) -> Result<impl IntoResponse, AppError> {
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
    _user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<MaintenanceForm>,
) -> Result<impl IntoResponse, AppError> {
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
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    db.call(move |conn| crate::db::maintenance::delete(conn, id))
        .await?;
    Ok(Redirect::to("/admin/maintenance"))
}

pub async fn close_maintenance(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
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

/// Check for maintenance windows starting within 1 hour and send reminder emails.
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

    if !config.get("email_enabled").is_some_and(|v| v == "true") {
        return;
    }

    for (id, name, start_time, end_time, endpoint_names, is_deploy, changelog) in &windows {
        let data = crate::notifications::email::MaintenanceReminderData {
            window_name: name.clone(),
            start_time: start_time.clone(),
            end_time: end_time.clone(),
            endpoint_names: endpoint_names.clone(),
            is_deploy: *is_deploy,
            changelog: changelog.clone(),
        };

        if let Err(e) = crate::notifications::email::send_maintenance_reminder(&config, &data).await
        {
            error!("Failed to send maintenance reminder for '{}': {e}", name);
            continue;
        }

        // Mark as sent
        let wid = *id;
        if let Err(e) = db
            .call(move |conn| crate::db::maintenance::mark_reminder_sent(conn, wid))
            .await
        {
            error!("Failed to mark reminder sent for window {id}: {e}");
        }
    }
}
