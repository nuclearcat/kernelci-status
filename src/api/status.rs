use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::auth::ApiAuth;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Serialize)]
pub struct StatusSummary {
    pub total: usize,
    pub ok: usize,
    pub warning: usize,
    pub critical: usize,
    pub no_data: usize,
}

pub async fn status(
    State(state): State<AppState>,
    _auth: ApiAuth,
) -> Result<Json<StatusSummary>, AppError> {
    let db = state.db.clone();
    let summary = db
        .call(|conn| {
            let eps = crate::db::endpoints::list_all(conn)?;
            let latest_by_endpoint = crate::db::history::get_latest_by_endpoint(conn)?;
            let mut ok = 0usize;
            let mut warning = 0usize;
            let mut critical = 0usize;
            let mut no_data = 0usize;
            for ep in &eps {
                match latest_by_endpoint.get(&ep.id).map(|h| h.state.as_str()) {
                    Some("OK") => ok += 1,
                    Some("WARNING") => warning += 1,
                    Some("CRITICAL") => critical += 1,
                    _ => no_data += 1,
                }
            }
            Ok(StatusSummary {
                total: eps.len(),
                ok,
                warning,
                critical,
                no_data,
            })
        })
        .await?;
    Ok(Json(summary))
}
