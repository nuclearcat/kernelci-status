use axum::Json;
use axum::extract::State;
use std::collections::HashMap;

use crate::auth::ApiAuth;
use crate::error::AppError;
use crate::state::AppState;

pub async fn get_config(
    State(state): State<AppState>,
    _auth: ApiAuth,
) -> Result<Json<HashMap<String, String>>, AppError> {
    let db = state.db.clone();
    let config = db
        .call(|conn| {
            let pairs = crate::db::config::get_all(conn)?;
            Ok(pairs.into_iter().collect::<HashMap<_, _>>())
        })
        .await?;
    let filtered: HashMap<String, String> = config
        .into_iter()
        .map(|(k, v)| {
            if k == "smtp_password" || k == "api_token" {
                (k, "********".to_string())
            } else {
                (k, v)
            }
        })
        .collect();
    Ok(Json(filtered))
}

pub async fn put_config(
    State(state): State<AppState>,
    _auth: ApiAuth,
    Json(updates): Json<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db = state.db.clone();
    db.call(move |conn| {
        for (key, value) in &updates {
            crate::db::config::set(conn, key, value)?;
        }
        Ok(())
    })
    .await?;

    // Update cache
    let db = state.db.clone();
    let new_config: HashMap<String, String> = db
        .call(|conn| {
            let pairs = crate::db::config::get_all(conn)?;
            Ok(pairs.into_iter().collect())
        })
        .await?;
    *state.config_cache.write().await = new_config;

    Ok(Json(serde_json::json!({ "ok": true })))
}
