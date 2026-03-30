use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::ApiAuth;
use crate::db::endpoints::{Endpoint, NewEndpoint};
use crate::db::history::HistoryEntry;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Serialize)]
pub struct EndpointWithState {
    #[serde(flatten)]
    endpoint: Endpoint,
    current_state: String,
    current_value: Option<String>,
    current_message: Option<String>,
    last_check: Option<String>,
}

pub async fn list_endpoints(
    State(state): State<AppState>,
    _auth: ApiAuth,
) -> Result<Json<Vec<EndpointWithState>>, AppError> {
    let db = state.db.clone();
    let result = db
        .call(|conn| {
            let eps = crate::db::endpoints::list_all(conn)?;
            let mut result = Vec::new();
            for ep in eps {
                let latest = crate::db::history::get_latest_for_endpoint(conn, ep.id)?;
                result.push(EndpointWithState {
                    current_state: latest
                        .as_ref()
                        .map(|h| h.state.clone())
                        .unwrap_or_else(|| "NO_DATA".to_string()),
                    current_value: latest.as_ref().and_then(|h| h.value.clone()),
                    current_message: latest.as_ref().and_then(|h| h.message.clone()),
                    last_check: latest.map(|h| h.timestamp),
                    endpoint: ep,
                });
            }
            Ok(result)
        })
        .await?;
    Ok(Json(result))
}

pub async fn get_endpoint(
    State(state): State<AppState>,
    _auth: ApiAuth,
    Path(id): Path<i64>,
) -> Result<Json<EndpointWithState>, AppError> {
    let db = state.db.clone();
    let result = db
        .call(move |conn| {
            let ep = crate::db::endpoints::get_by_id(conn, id)?
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            let latest = crate::db::history::get_latest_for_endpoint(conn, id)?;
            Ok(EndpointWithState {
                current_state: latest
                    .as_ref()
                    .map(|h| h.state.clone())
                    .unwrap_or_else(|| "NO_DATA".to_string()),
                current_value: latest.as_ref().and_then(|h| h.value.clone()),
                current_message: latest.as_ref().and_then(|h| h.message.clone()),
                last_check: latest.map(|h| h.timestamp),
                endpoint: ep,
            })
        })
        .await?;
    Ok(Json(result))
}

#[derive(Deserialize)]
pub struct CreateEndpointRequest {
    pub name: String,
    pub subname: Option<String>,
    pub endpoint: String,
    pub check_type: String,
    pub selector: Option<String>,
    pub condition: Option<String>,
    pub critical: Option<bool>,
    pub enabled: Option<bool>,
}

pub async fn create_endpoint(
    State(state): State<AppState>,
    _auth: ApiAuth,
    Json(req): Json<CreateEndpointRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ep = NewEndpoint {
        name: req.name,
        subname: req.subname,
        endpoint: req.endpoint,
        check_type: req.check_type,
        selector: req.selector,
        condition: req.condition,
        critical: req.critical.unwrap_or(false),
        enabled: req.enabled.unwrap_or(true),
    };
    let db = state.db.clone();
    let id = db
        .call(move |conn| crate::db::endpoints::insert(conn, &ep))
        .await?;
    Ok(Json(serde_json::json!({ "id": id })))
}

pub async fn update_endpoint(
    State(state): State<AppState>,
    _auth: ApiAuth,
    Path(id): Path<i64>,
    Json(req): Json<CreateEndpointRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ep = NewEndpoint {
        name: req.name,
        subname: req.subname,
        endpoint: req.endpoint,
        check_type: req.check_type,
        selector: req.selector,
        condition: req.condition,
        critical: req.critical.unwrap_or(false),
        enabled: req.enabled.unwrap_or(true),
    };
    let db = state.db.clone();
    let updated = db
        .call(move |conn| crate::db::endpoints::update(conn, id, &ep))
        .await?;
    if !updated {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn delete_endpoint(
    State(state): State<AppState>,
    _auth: ApiAuth,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db = state.db.clone();
    let deleted = db
        .call(move |conn| crate::db::endpoints::delete(conn, id))
        .await?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize, Default)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn endpoint_history(
    State(state): State<AppState>,
    _auth: ApiAuth,
    Path(id): Path<i64>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<HistoryEntry>>, AppError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 1000);
    let offset = query.offset.unwrap_or(0).max(0);
    let db = state.db.clone();
    let entries = db
        .call(move |conn| crate::db::history::get_for_endpoint(conn, id, limit, offset))
        .await?;
    Ok(Json(entries))
}
