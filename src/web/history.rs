use askama::Template;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse};
use serde::Deserialize;
use std::collections::HashMap;
use tokio::io::AsyncWriteExt;

use crate::auth::AuthUser;
use crate::db::endpoints::Endpoint;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct HistoryQuery {
    pub endpoint_id: Option<i64>,
    pub state: Option<String>,
    pub page: Option<i64>,
}

/// Display-ready history entry with pre-resolved fields.
pub struct HistoryDisplay {
    pub timestamp: String,
    pub endpoint_name: String,
    pub state: String,
    pub display_value: String,
    pub display_message: String,
}

#[derive(Template)]
#[template(path = "history.html")]
struct HistoryTemplate {
    username: String,
    entries: Vec<HistoryDisplay>,
    endpoints: Vec<Endpoint>,
    current_endpoint_id: i64,
    current_state_str: String,
    page: i64,
    total_pages: i64,
    filter_query: String,
}

#[derive(Template)]
#[template(path = "fragments/history_table.html")]
struct HistoryTableTemplate {
    entries: Vec<HistoryDisplay>,
    page: i64,
    total_pages: i64,
    filter_query: String,
}

pub async fn history_page(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<HistoryQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let page = query.page.unwrap_or(1).max(1);
    let per_page = 50i64;
    let offset = (page - 1) * per_page;

    let state_filter = query.state.clone().filter(|s| !s.is_empty());
    let endpoint_filter = query.endpoint_id;

    let db = state.db.clone();
    let sf = state_filter.clone();
    let (raw_entries, total, endpoints) = db
        .call(move |conn| {
            let entries = crate::db::history::get_all(
                conn,
                per_page,
                offset,
                endpoint_filter,
                sf.as_deref(),
            )?;
            let total = crate::db::history::count_all(
                conn,
                endpoint_filter,
                state_filter.as_deref(),
            )?;
            let endpoints = crate::db::endpoints::list_all(conn)?;
            Ok((entries, total, endpoints))
        })
        .await?;

    let total_pages = ((total as f64) / (per_page as f64)).ceil().max(1.0) as i64;

    let endpoint_names: HashMap<i64, String> = endpoints
        .iter()
        .map(|e| {
            let name = match &e.subname {
                Some(sub) => format!("{} ({})", e.name, sub),
                None => e.name.clone(),
            };
            (e.id, name)
        })
        .collect();

    let entries: Vec<HistoryDisplay> = raw_entries
        .into_iter()
        .map(|e| {
            let endpoint_name = endpoint_names
                .get(&e.endpoint_id)
                .cloned()
                .unwrap_or_else(|| "Unknown".to_string());
            HistoryDisplay {
                timestamp: e.timestamp,
                endpoint_name,
                state: e.state,
                display_value: e.value.unwrap_or_else(|| "-".to_string()),
                display_message: e.message.unwrap_or_else(|| "-".to_string()),
            }
        })
        .collect();

    // Build filter query string for pagination links
    let mut filter_query = String::new();
    if let Some(eid) = query.endpoint_id {
        filter_query.push_str(&format!("&endpoint_id={eid}"));
    }
    if let Some(ref st) = query.state {
        if !st.is_empty() {
            let encoded: String = st
                .bytes()
                .flat_map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        vec![b as char]
                    }
                    _ => format!("%{b:02X}").chars().collect(),
                })
                .collect();
            filter_query.push_str(&format!("&state={encoded}"));
        }
    }

    let is_htmx = headers.get("HX-Request").is_some();

    if is_htmx {
        Ok(Html(
            HistoryTableTemplate {
                entries,
                page,
                total_pages,
                filter_query,
            }
            .render()
            .unwrap_or_default(),
        ))
    } else {
        Ok(Html(
            HistoryTemplate {
                username: user.username,
                entries,
                endpoints,
                current_endpoint_id: query.endpoint_id.unwrap_or(0),
                current_state_str: query.state.unwrap_or_default(),
                page,
                total_pages,
                filter_query,
            }
            .render()
            .unwrap_or_default(),
        ))
    }
}

pub async fn export_old(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();

    let total = db
        .call(|conn| crate::db::history::count_old_entries(conn, 2))
        .await?;

    if total == 0 {
        return Ok(Html("No entries older than 2 months to export.".to_string()));
    }

    let filename = format!(
        "state_history_export_{}.sql",
        chrono::Utc::now().format("%Y%m%d_%H%M%S")
    );

    // Write in batches of 5000 rows to avoid loading everything into memory
    let batch_size: i64 = 5000;
    let mut offset: i64 = 0;
    let mut first = true;

    while offset < total {
        let db = state.db.clone();
        let batch = db
            .call(move |conn| {
                crate::db::history::export_old_entries_batch(conn, 2, batch_size, offset)
            })
            .await?;

        if batch.is_empty() {
            break;
        }

        if first {
            tokio::fs::write(&filename, &batch)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to write export: {e}")))?;
            first = false;
        } else {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(&filename)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to open export file: {e}")))?
                .write_all(batch.as_bytes())
                .await
                .map_err(|e| AppError::Internal(format!("Failed to append export: {e}")))?;
        }

        offset += batch_size;
    }

    // Delete after successful export
    let deleted = db
        .call(|conn| crate::db::history::delete_old_entries(conn, 2))
        .await?;

    Ok(Html(format!(
        "Exported {total} and deleted {deleted} old entries. File: {filename}"
    )))
}
