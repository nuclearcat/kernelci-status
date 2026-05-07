use askama::Template;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse};

use crate::auth::AuthUser;
use crate::db::endpoints::{Endpoint, EndpointWithState};
use crate::db::history::HistoryEntry;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    username: String,
    endpoints: Vec<EndpointWithState>,
    critical_count: usize,
    warning_count: usize,
    ok_count: usize,
    nodata_count: usize,
}

#[derive(Template)]
#[template(path = "fragments/endpoint_row.html")]
struct EndpointRowsTemplate {
    endpoints: Vec<EndpointWithState>,
}

pub async fn dashboard(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let endpoints: Vec<EndpointWithState> = db
        .call(|conn| crate::db::endpoints::list_all_with_latest_state(conn))
        .await?;

    let critical_count = endpoints
        .iter()
        .filter(|e| e.state == "CRITICAL" || e.state == "CRITICAL_MAINTENANCE")
        .count();
    let warning_count = endpoints
        .iter()
        .filter(|e| e.state == "WARNING" || e.state == "WARNING_MAINTENANCE")
        .count();
    let ok_count = endpoints
        .iter()
        .filter(|e| e.state == "OK" || e.state == "OK_MAINTENANCE")
        .count();
    let nodata_count = endpoints
        .iter()
        .filter(|e| {
            e.state == "NO_DATA" || e.state == "NO_DATA_MAINTENANCE" || e.state == "MAINTENANCE"
        })
        .count();

    let is_htmx = headers.get("HX-Request").is_some();
    if is_htmx {
        Ok(Html(
            EndpointRowsTemplate { endpoints }
                .render()
                .unwrap_or_default(),
        ))
    } else {
        Ok(Html(
            DashboardTemplate {
                username: user.username,
                endpoints,
                critical_count,
                warning_count,
                ok_count,
                nodata_count,
            }
            .render()
            .unwrap_or_default(),
        ))
    }
}

#[derive(Template)]
#[template(path = "endpoint_detail.html")]
struct EndpointDetailTemplate {
    username: String,
    endpoint: Endpoint,
    current_state: String,
    current_value: Option<String>,
    current_message: Option<String>,
    history: Vec<HistoryEntry>,
}

pub async fn endpoint_detail(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let (endpoint, history) = db
        .call(move |conn| {
            let ep = crate::db::endpoints::get_by_id(conn, id)?
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            let hist = crate::db::history::get_for_endpoint(conn, id, 50, 0)?;
            Ok((ep, hist))
        })
        .await?;

    let latest = history.first();
    Ok(Html(
        EndpointDetailTemplate {
            username: user.username,
            current_state: latest
                .map(|h| h.state.clone())
                .unwrap_or_else(|| "NO_DATA".to_string()),
            current_value: latest.and_then(|h| h.value.clone()),
            current_message: latest.and_then(|h| h.message.clone()),
            endpoint,
            history,
        }
        .render()
        .unwrap_or_default(),
    ))
}
