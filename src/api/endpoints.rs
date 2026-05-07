// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};

use crate::auth::ApiAuth;
use crate::db::endpoints::{Endpoint, EndpointWithState, NewEndpoint};
use crate::db::history::HistoryEntry;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Serialize)]
pub struct EndpointResponse {
    #[serde(flatten)]
    endpoint: Endpoint,
    current_state: String,
    current_value: Option<String>,
    current_message: Option<String>,
    last_check: Option<String>,
}

impl From<EndpointWithState> for EndpointResponse {
    fn from(endpoint: EndpointWithState) -> Self {
        Self {
            endpoint: endpoint.endpoint,
            current_state: endpoint.state,
            current_value: endpoint.value,
            current_message: endpoint.message,
            last_check: endpoint.last_check,
        }
    }
}

pub async fn list_endpoints(
    State(state): State<AppState>,
    _auth: ApiAuth,
) -> Result<Json<Vec<EndpointResponse>>, AppError> {
    let db = state.db.clone();
    let result = db
        .call(|conn| {
            Ok(crate::db::endpoints::list_all_with_latest_state(conn)?
                .into_iter()
                .map(EndpointResponse::from)
                .collect())
        })
        .await?;
    Ok(Json(result))
}

pub async fn get_endpoint(
    State(state): State<AppState>,
    _auth: ApiAuth,
    Path(id): Path<i64>,
) -> Result<Json<EndpointResponse>, AppError> {
    let db = state.db.clone();
    let result = db
        .call(move |conn| {
            let ep = crate::db::endpoints::get_by_id(conn, id)?
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            let latest = crate::db::history::get_latest_for_endpoint(conn, id)?;
            Ok(EndpointResponse::from(EndpointWithState::new(
                ep,
                latest.as_ref(),
            )))
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
    pub nodata_behavior: Option<String>,
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
        nodata_behavior: match req.nodata_behavior.as_deref() {
            Some("warning") => "warning".to_string(),
            Some("critical") => "critical".to_string(),
            _ => "nodata".to_string(),
        },
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
        nodata_behavior: match req.nodata_behavior.as_deref() {
            Some("warning") => "warning".to_string(),
            Some("critical") => "critical".to_string(),
            _ => "nodata".to_string(),
        },
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
