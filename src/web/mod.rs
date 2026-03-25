pub mod configuration;
pub mod dashboard;
pub mod endpoints;
pub mod help;
pub mod history;
pub mod login;
pub mod notifications;
pub mod status;
pub mod users;

use axum::http::{header, HeaderValue};
use axum::response::IntoResponse;
use axum::Router;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::state::AppState;

// Embed static files at compile time
static HTMX_JS: &[u8] = include_bytes!("../../static/htmx.min.js");
static PICO_CSS: &[u8] = include_bytes!("../../static/pico.min.css");
static STYLE_CSS: &[u8] = include_bytes!("../../static/style.css");

async fn serve_htmx() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/javascript")], HTMX_JS)
}

async fn serve_pico() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], PICO_CSS)
}

async fn serve_style() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], STYLE_CSS)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        // Public status page (no auth)
        .route("/", axum::routing::get(status::status_page))
        .route("/status", axum::routing::get(status::status_page))
        .route("/status/data", axum::routing::get(status::status_data))
        // Login/logout (public)
        .route("/login", axum::routing::get(login::login_page))
        .route("/login", axum::routing::post(login::login_submit))
        .route("/logout", axum::routing::post(login::logout))
        // Admin routes (auth required)
        .route("/admin", axum::routing::get(dashboard::dashboard))
        .route(
            "/admin/endpoints/{id}",
            axum::routing::get(dashboard::endpoint_detail),
        )
        .route(
            "/admin/endpoints",
            axum::routing::get(endpoints::endpoints_page),
        )
        .route(
            "/admin/endpoints/add",
            axum::routing::post(endpoints::add_endpoint),
        )
        .route(
            "/admin/endpoints/edit/{id}",
            axum::routing::post(endpoints::edit_endpoint),
        )
        .route(
            "/admin/endpoints/delete/{id}",
            axum::routing::post(endpoints::delete_endpoint),
        )
        .route(
            "/admin/notifications",
            axum::routing::get(notifications::notifications_page),
        )
        .route(
            "/admin/notifications",
            axum::routing::post(notifications::save_notifications),
        )
        .route(
            "/admin/configuration",
            axum::routing::get(configuration::configuration_page),
        )
        .route(
            "/admin/configuration",
            axum::routing::post(configuration::save_configuration),
        )
        .route(
            "/admin/configuration/test-email",
            axum::routing::post(configuration::test_email),
        )
        .route("/admin/users", axum::routing::get(users::users_page))
        .route("/admin/users/add", axum::routing::post(users::add_user))
        .route(
            "/admin/users/password/{id}",
            axum::routing::post(users::change_password),
        )
        .route(
            "/admin/users/delete/{id}",
            axum::routing::post(users::delete_user),
        )
        .route(
            "/admin/history",
            axum::routing::get(history::history_page),
        )
        .route(
            "/admin/history/export",
            axum::routing::post(history::export_old),
        )
        .route("/admin/help", axum::routing::get(help::help_page))
        // Embedded static files
        .route("/static/htmx.min.js", axum::routing::get(serve_htmx))
        .route("/static/pico.min.css", axum::routing::get(serve_pico))
        .route("/static/style.css", axum::routing::get(serve_style))
        // API routes
        .nest("/api", crate::api::router())
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("x-xss-protection"),
            HeaderValue::from_static("1; mode=block"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'none'"),
        ))
        .with_state(state)
}
