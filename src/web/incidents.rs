use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect};
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::db::endpoints::Endpoint;
use crate::db::incidents::{Incident, IncidentUpdate};
use crate::db::users::User;
use crate::error::AppError;
use crate::state::AppState;

// ── Token generation helper ──

fn generate_token() -> String {
    use rand::RngExt;
    rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect()
}

fn token_expiry() -> String {
    (chrono::Utc::now() + chrono::Duration::hours(24))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

// ── Admin: list page ──

#[derive(Template)]
#[template(path = "incidents.html")]
struct IncidentsTemplate {
    username: String,
    incidents: Vec<IncidentRow>,
    endpoints: Vec<Endpoint>,
}

pub struct IncidentRow {
    pub incident: Incident,
    pub endpoint_name: String,
    pub assigned_username: Option<String>,
}

pub async fn incidents_page(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let (incidents, endpoints) = db
        .call(|conn| {
            let incs = crate::db::incidents::list_all(conn)?;
            let eps = crate::db::endpoints::list_all(conn)?;
            let users = crate::db::users::list_all(conn)?;

            let rows: Vec<IncidentRow> = incs
                .into_iter()
                .map(|inc| {
                    let ep_name = eps
                        .iter()
                        .find(|e| e.id == inc.endpoint_id)
                        .map(|e| match &e.subname {
                            Some(sub) => format!("{} ({})", e.name, sub),
                            None => e.name.clone(),
                        })
                        .unwrap_or_else(|| format!("Endpoint #{}", inc.endpoint_id));
                    let assigned = inc
                        .assigned_user_id
                        .and_then(|uid| users.iter().find(|u| u.id == uid))
                        .map(|u| u.username.clone());
                    IncidentRow {
                        incident: inc,
                        endpoint_name: ep_name,
                        assigned_username: assigned,
                    }
                })
                .collect();
            Ok((rows, eps))
        })
        .await?;

    Ok(Html(
        IncidentsTemplate {
            username: user.username,
            incidents,
            endpoints,
        }
        .render()
        .unwrap_or_default(),
    ))
}

// ── Admin: detail page ──

#[derive(Template)]
#[template(path = "incident_detail.html")]
struct IncidentDetailTemplate {
    username: String,
    incident: Incident,
    endpoint_name: String,
    assigned_username: Option<String>,
    updates: Vec<UpdateRow>,
    users: Vec<User>,
}

pub struct UpdateRow {
    pub update: IncidentUpdate,
    pub username: Option<String>,
}

pub async fn incident_detail(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let result = db
        .call(move |conn| {
            let inc = crate::db::incidents::get_by_id(conn, id)?;
            let inc = match inc {
                Some(i) => i,
                None => return Ok(None),
            };
            let eps = crate::db::endpoints::list_all(conn)?;
            let users = crate::db::users::list_all(conn)?;
            let updates = crate::db::incidents::get_updates(conn, id)?;

            let ep_name = eps
                .iter()
                .find(|e| e.id == inc.endpoint_id)
                .map(|e| match &e.subname {
                    Some(sub) => format!("{} ({})", e.name, sub),
                    None => e.name.clone(),
                })
                .unwrap_or_else(|| format!("Endpoint #{}", inc.endpoint_id));

            let assigned = inc
                .assigned_user_id
                .and_then(|uid| users.iter().find(|u| u.id == uid))
                .map(|u| u.username.clone());

            let update_rows: Vec<UpdateRow> = updates
                .into_iter()
                .map(|u| {
                    let uname = u
                        .user_id
                        .and_then(|uid| users.iter().find(|usr| usr.id == uid))
                        .map(|usr| usr.username.clone());
                    UpdateRow {
                        update: u,
                        username: uname,
                    }
                })
                .collect();

            Ok(Some((inc, ep_name, assigned, update_rows, users)))
        })
        .await?;

    match result {
        Some((incident, endpoint_name, assigned_username, updates, users)) => Ok(Html(
            IncidentDetailTemplate {
                username: user.username,
                incident,
                endpoint_name,
                assigned_username,
                updates,
                users,
            }
            .render()
            .unwrap_or_default(),
        )),
        None => Err(AppError::NotFound),
    }
}

// ── Admin: create incident ──

#[derive(Deserialize)]
pub struct CreateIncidentForm {
    pub endpoint_id: i64,
    pub title: String,
    pub severity: String,
    pub public_message: Option<String>,
}

pub async fn create_incident(
    State(state): State<AppState>,
    _user: AuthUser,
    Form(form): Form<CreateIncidentForm>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    db.call(move |conn| {
        let new = crate::db::incidents::NewIncident {
            endpoint_id: form.endpoint_id,
            title: form.title.clone(),
            severity: form.severity.clone(),
            auto_created: false,
        };
        let id = crate::db::incidents::insert(conn, &new)?;
        crate::db::incidents::add_update(
            conn,
            id,
            "status_change",
            Some("detected"),
            Some("Incident created manually"),
            None,
        )?;
        if let Some(msg) = &form.public_message {
            if !msg.trim().is_empty() {
                crate::db::incidents::save_public_message(conn, id, msg.trim())?;
            }
        }
        Ok(())
    })
    .await?;

    Ok(Redirect::to("/admin/incidents"))
}

// ── Admin: update status ──

#[derive(Deserialize)]
pub struct UpdateStatusForm {
    pub status: String,
}

pub async fn update_incident_status(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<UpdateStatusForm>,
) -> Result<impl IntoResponse, AppError> {
    let user_id = auth.user_id;
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::incidents::update_status(conn, id, &form.status)?;
        crate::db::incidents::add_update(
            conn,
            id,
            "status_change",
            Some(&form.status),
            None,
            Some(user_id),
        )?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to(&format!("/admin/incidents/{id}")))
}

// ── Admin: add comment ──

#[derive(Deserialize)]
pub struct CommentForm {
    pub message: String,
}

pub async fn add_comment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<CommentForm>,
) -> Result<impl IntoResponse, AppError> {
    if form.message.trim().is_empty() {
        return Err(AppError::BadRequest("Comment cannot be empty".to_string()));
    }
    let user_id = auth.user_id;
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::incidents::add_update(
            conn,
            id,
            "comment",
            None,
            Some(form.message.trim()),
            Some(user_id),
        )?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to(&format!("/admin/incidents/{id}")))
}

// ── Admin: update public message ──

#[derive(Deserialize)]
pub struct PublicMessageForm {
    pub public_message: String,
}

pub async fn update_public_message(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<PublicMessageForm>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::incidents::save_public_message(conn, id, form.public_message.trim())?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to(&format!("/admin/incidents/{id}")))
}

// ── Admin: handover ──

#[derive(Deserialize)]
pub struct HandoverForm {
    pub user_id: i64,
}

pub async fn handover_incident(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<HandoverForm>,
) -> Result<impl IntoResponse, AppError> {
    let from_user_id = auth.user_id;
    let to_user_id = form.user_id;

    let db = state.db.clone();
    let (incident, to_user, endpoint_name, config) = db
        .call(move |conn| {
            let inc = crate::db::incidents::get_by_id(conn, id)?
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            let to_user = crate::db::users::get_by_id(conn, to_user_id)?
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            let eps = crate::db::endpoints::list_all(conn)?;
            let ep_name = eps
                .iter()
                .find(|e| e.id == inc.endpoint_id)
                .map(|e| e.name.clone())
                .unwrap_or_default();

            crate::db::incidents::assign(conn, id, to_user_id)?;
            let from_user = crate::db::users::get_by_id(conn, from_user_id)?;
            let from_name = from_user
                .map(|u| u.username)
                .unwrap_or_else(|| "Unknown".to_string());
            let msg = format!("Handed over from {} to {}", from_name, to_user.username);
            crate::db::incidents::add_update(
                conn,
                id,
                "handover",
                None,
                Some(&msg),
                Some(from_user_id),
            )?;

            // Generate action tokens for the new assignee
            let expires = token_expiry();
            for action in &["acknowledge", "investigating", "identified"] {
                let tok = generate_token();
                crate::db::incidents::create_token(conn, id, to_user_id, action, &tok, &expires)?;
            }

            let config_pairs = crate::db::config::get_all(conn)?;
            let config: std::collections::HashMap<String, String> =
                config_pairs.into_iter().collect();

            Ok((inc, to_user, ep_name, config))
        })
        .await?;

    // Send email to the new assignee
    if let Some(email) = &to_user.email {
        if !email.is_empty() {
            let base_url = config.get("base_url").cloned().unwrap_or_default();
            // Fetch their tokens
            let db = state.db.clone();
            let inc_id = incident.id;
            let uid = to_user.id;
            let tokens = db
                .call(move |conn| -> rusqlite::Result<Vec<(String, String)>> {
                    let mut stmt = conn.prepare(
                        "SELECT action, token FROM incident_tokens \
                         WHERE incident_id = ?1 AND user_id = ?2 AND used_at IS NULL \
                         ORDER BY id DESC LIMIT 3",
                    )?;
                    let rows: Vec<(String, String)> = stmt
                        .query_map(rusqlite::params![inc_id, uid], |row| {
                            Ok((row.get(0)?, row.get(1)?))
                        })?
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                })
                .await
                .unwrap_or_default();

            let action_links: Vec<crate::notifications::email::ActionLink> = tokens
                .iter()
                .map(|(action, tok)| crate::notifications::email::ActionLink {
                    label: format!("Mark as {}", action.replace('_', " ")),
                    url: format!("{}/incident/action/{}", base_url, tok),
                })
                .collect();

            let incident_data = crate::notifications::email::IncidentEmailData {
                title: incident.title.clone(),
                endpoint_name: endpoint_name.clone(),
                severity: incident.severity.clone(),
                status: incident.status.clone(),
            };

            let subject = format!("[Incident Handover] {}", incident.title);
            let _ = crate::notifications::email::send_incident_email(
                &config,
                email,
                &to_user.username,
                &subject,
                &incident_data,
                &action_links,
                "This incident has been handed over to you. Please use the buttons below to update the status.",
            )
            .await;
        }
    }

    Ok(Redirect::to(&format!("/admin/incidents/{id}")))
}

// ── Admin: save postmortem ──

#[derive(Deserialize)]
pub struct PostmortemForm {
    pub postmortem: String,
}

pub async fn save_postmortem(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<PostmortemForm>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::incidents::save_postmortem(conn, id, form.postmortem.trim())?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to(&format!("/admin/incidents/{id}")))
}

// ── Public: token action ──

#[derive(Template)]
#[template(path = "incident_action.html")]
struct IncidentActionTemplate {
    success: bool,
    title: String,
    message: String,
}

pub async fn incident_token_action(
    State(state): State<AppState>,
    Path(token_str): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let tok_str = token_str.clone();

    let result = db
        .call(move |conn| {
            let token = crate::db::incidents::get_valid_token(conn, &tok_str)?;
            let token = match token {
                Some(t) => t,
                None => return Ok(None),
            };

            let incident = crate::db::incidents::get_by_id(conn, token.incident_id)?;
            let incident = match incident {
                Some(i) => i,
                None => return Ok(None),
            };

            let user = crate::db::users::get_by_id(conn, token.user_id)?;
            let username = user
                .as_ref()
                .map(|u| u.username.clone())
                .unwrap_or_default();
            let user_email = user.as_ref().and_then(|u| u.email.clone());

            // Perform the action
            match token.action.as_str() {
                "acknowledge" => {
                    crate::db::incidents::update_status(conn, incident.id, "acknowledged")?;
                    crate::db::incidents::assign(conn, incident.id, token.user_id)?;
                    crate::db::incidents::add_update(
                        conn,
                        incident.id,
                        "status_change",
                        Some("acknowledged"),
                        Some(&format!("Acknowledged by {}", username)),
                        Some(token.user_id),
                    )?;

                    // Generate investigating + identified tokens for this user
                    let expires = token_expiry();
                    for action in &["investigating", "identified"] {
                        let tok = generate_token();
                        crate::db::incidents::create_token(
                            conn,
                            incident.id,
                            token.user_id,
                            action,
                            &tok,
                            &expires,
                        )?;
                    }
                }
                "investigating" => {
                    crate::db::incidents::update_status(conn, incident.id, "investigating")?;
                    crate::db::incidents::add_update(
                        conn,
                        incident.id,
                        "status_change",
                        Some("investigating"),
                        Some(&format!("{} is investigating", username)),
                        Some(token.user_id),
                    )?;
                }
                "identified" => {
                    crate::db::incidents::update_status(conn, incident.id, "identified")?;
                    crate::db::incidents::add_update(
                        conn,
                        incident.id,
                        "status_change",
                        Some("identified"),
                        Some(&format!("Issue identified by {}", username)),
                        Some(token.user_id),
                    )?;
                }
                _ => {}
            }

            crate::db::incidents::mark_token_used(conn, &token.token)?;

            // Gather data for follow-up email
            let config_pairs = crate::db::config::get_all(conn)?;
            let config: std::collections::HashMap<String, String> =
                config_pairs.into_iter().collect();

            let eps = crate::db::endpoints::list_all(conn)?;
            let ep_name = eps
                .iter()
                .find(|e| e.id == incident.endpoint_id)
                .map(|e| e.name.clone())
                .unwrap_or_default();

            // Get fresh tokens for the user (for follow-up email)
            let mut stmt = conn.prepare(
                "SELECT action, token FROM incident_tokens \
                 WHERE incident_id = ?1 AND user_id = ?2 AND used_at IS NULL \
                 AND expires_at > datetime('now') ORDER BY id DESC",
            )?;
            let fresh_tokens: Vec<(String, String)> = stmt
                .query_map(rusqlite::params![incident.id, token.user_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(Some((
                incident,
                token.action.clone(),
                username,
                user_email,
                config,
                ep_name,
                fresh_tokens,
            )))
        })
        .await?;

    match result {
        None => Ok(Html(
            IncidentActionTemplate {
                success: false,
                title: "Invalid or Expired Link".to_string(),
                message: "This action link is invalid, has already been used, or has expired."
                    .to_string(),
            }
            .render()
            .unwrap_or_default(),
        )),
        Some((incident, action, username, user_email, config, ep_name, fresh_tokens)) => {
            // Send follow-up email with remaining action links (only on acknowledge)
            if action == "acknowledge" {
                if let Some(email) = &user_email {
                    if !email.is_empty() {
                        let base_url = config.get("base_url").cloned().unwrap_or_default();
                        let action_links: Vec<crate::notifications::email::ActionLink> =
                            fresh_tokens
                                .iter()
                                .map(|(a, tok)| crate::notifications::email::ActionLink {
                                    label: format!("Mark as {}", a.replace('_', " ")),
                                    url: format!("{}/incident/action/{}", base_url, tok),
                                })
                                .collect();

                        let incident_data = crate::notifications::email::IncidentEmailData {
                            title: incident.title.clone(),
                            endpoint_name: ep_name,
                            severity: incident.severity.clone(),
                            status: "acknowledged".to_string(),
                        };

                        let _ = crate::notifications::email::send_incident_email(
                            &config,
                            email,
                            &username,
                            &format!("[Incident] {} - Next Steps", incident.title),
                            &incident_data,
                            &action_links,
                            "You have acknowledged this incident. Use the buttons below to update its status as you investigate.",
                        )
                        .await;
                    }
                }
            }

            let msg = match action.as_str() {
                "acknowledge" => format!(
                    "You ({}) have acknowledged incident \"{}\". Check your email for next action links.",
                    username, incident.title
                ),
                "investigating" => format!(
                    "Incident \"{}\" marked as investigating by {}.",
                    incident.title, username
                ),
                "identified" => format!(
                    "Issue identified for incident \"{}\" by {}.",
                    incident.title, username
                ),
                _ => "Action completed.".to_string(),
            };

            Ok(Html(
                IncidentActionTemplate {
                    success: true,
                    title: format!("Incident {}", action.replace('_', " ")),
                    message: msg,
                }
                .render()
                .unwrap_or_default(),
            ))
        }
    }
}

// ── Helper: send incident emails to all admins (used by scheduler) ──

pub async fn send_incident_created_emails(
    state: &AppState,
    incident_id: i64,
    incident_title: &str,
    endpoint_name: &str,
    severity: &str,
) {
    let db = state.db.clone();
    let title = incident_title.to_string();
    let ep_name = endpoint_name.to_string();
    let sev = severity.to_string();

    type EmailResult = (
        std::collections::HashMap<String, String>,
        Vec<(String, String, String)>,
        String,
        String,
        String,
    );
    let result: Result<EmailResult, _> = db
        .call(move |conn| -> rusqlite::Result<EmailResult> {
            let users = crate::db::users::list_with_email(conn)?;
            let config_pairs = crate::db::config::get_all(conn)?;
            let config: std::collections::HashMap<String, String> =
                config_pairs.into_iter().collect();

            let expires = token_expiry();
            let mut user_tokens: Vec<(String, String, String)> = Vec::new(); // (email, username, token)

            for user in &users {
                let tok = generate_token();
                crate::db::incidents::create_token(
                    conn,
                    incident_id,
                    user.id,
                    "acknowledge",
                    &tok,
                    &expires,
                )?;
                if let Some(email) = &user.email {
                    user_tokens.push((email.clone(), user.username.clone(), tok));
                }
            }

            Ok((config, user_tokens, title, ep_name, sev))
        })
        .await;

    let (config, user_tokens, title, ep_name, severity) = match result {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to prepare incident emails: {e}");
            return;
        }
    };

    let base_url = config.get("base_url").cloned().unwrap_or_default();

    for (email, username, token) in user_tokens {
        let incident_data = crate::notifications::email::IncidentEmailData {
            title: title.clone(),
            endpoint_name: ep_name.clone(),
            severity: severity.clone(),
            status: "detected".to_string(),
        };

        let action_links = vec![crate::notifications::email::ActionLink {
            label: "Acknowledge Incident".to_string(),
            url: format!("{}/incident/action/{}", base_url, token),
        }];

        let subject = format!("[INCIDENT] {}", title);
        if let Err(e) = crate::notifications::email::send_incident_email(
            &config,
            &email,
            &username,
            &subject,
            &incident_data,
            &action_links,
            "A new incident has been detected. Click below to acknowledge and take ownership.",
        )
        .await
        {
            tracing::error!("Failed to send incident email to {email}: {e}");
        }
    }
}

/// Re-send escalation emails for unacknowledged incidents.
pub async fn check_escalations(state: &AppState) {
    let db = state.db.clone();

    let escalation_minutes: i64 = {
        let cache = state.config_cache.read().await;
        cache
            .get("incident_escalation_minutes")
            .and_then(|v| v.parse().ok())
            .unwrap_or(30)
    };

    let incidents = db
        .call(move |conn| {
            crate::db::incidents::get_unacknowledged_past_threshold(conn, escalation_minutes)
        })
        .await;

    let incidents = match incidents {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("Failed to check escalations: {e}");
            return;
        }
    };

    for incident in incidents {
        let db = state.db.clone();
        let inc_id = incident.id;
        let ep_id = incident.endpoint_id;

        let ep_name = db
            .call(move |conn| -> rusqlite::Result<String> {
                let ep = crate::db::endpoints::list_all(conn)?;
                Ok(ep
                    .iter()
                    .find(|e| e.id == ep_id)
                    .map(|e| e.name.clone())
                    .unwrap_or_default())
            })
            .await
            .unwrap_or_default();

        tracing::info!(
            "Escalating incident #{} '{}' (unacknowledged for >{} min)",
            inc_id,
            incident.title,
            escalation_minutes
        );

        // Mark before sending so an incident that already got escalated won't
        // be re-sent on the next scheduler tick.
        let marked = state
            .db
            .clone()
            .call(move |conn| crate::db::incidents::mark_escalated(conn, inc_id))
            .await
            .unwrap_or(false);
        if !marked {
            continue;
        }

        send_incident_created_emails(state, inc_id, &incident.title, &ep_name, &incident.severity)
            .await;
    }
}
