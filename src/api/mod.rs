pub mod config;
pub mod endpoints;
pub mod status;

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/endpoints", axum::routing::get(endpoints::list_endpoints))
        .route(
            "/endpoints",
            axum::routing::post(endpoints::create_endpoint),
        )
        .route(
            "/endpoints/{id}",
            axum::routing::get(endpoints::get_endpoint),
        )
        .route(
            "/endpoints/{id}",
            axum::routing::put(endpoints::update_endpoint),
        )
        .route(
            "/endpoints/{id}",
            axum::routing::delete(endpoints::delete_endpoint),
        )
        .route(
            "/endpoints/{id}/history",
            axum::routing::get(endpoints::endpoint_history),
        )
        .route("/status", axum::routing::get(status::status))
        .route("/config", axum::routing::get(config::get_config))
        .route("/config", axum::routing::put(config::put_config))
}
