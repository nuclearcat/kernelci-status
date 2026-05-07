// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

pub mod common;
pub mod configuration;
pub mod dashboard;
pub mod endpoints;
pub mod help;
pub mod history;
pub mod incidents;
pub mod login;
pub mod maintenance;
pub mod notifications;
pub mod reports;
pub mod status;
pub mod users;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;
use tower_http::set_header::SetResponseHeaderLayer;

const RESTORE_BODY_LIMIT: usize = 128 * 1024 * 1024;

use crate::state::AppState;

// Embed static files at compile time
static HTMX_JS: &[u8] = include_bytes!("../../static/htmx.min.js");
static PICO_CSS: &[u8] = include_bytes!("../../static/pico.min.css");
static STYLE_CSS: &[u8] = include_bytes!("../../static/style.css");
static LOGO_SVG: &[u8] = include_bytes!("../../static/kernelci-horizontal-color-1.svg");

async fn serve_htmx() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/javascript")], HTMX_JS)
}

async fn serve_pico() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], PICO_CSS)
}

async fn serve_style() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], STYLE_CSS)
}

async fn serve_logo() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/svg+xml")], LOGO_SVG)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        // Public status page (no auth)
        .route("/", axum::routing::get(status::status_page))
        .route("/status", axum::routing::get(status::status_page))
        .route("/status/data", axum::routing::get(status::status_data))
        // Public incident token actions (no auth)
        .route("/incident/action/{token}", axum::routing::get(incidents::incident_token_action))
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
            "/admin/endpoints/clone/{id}",
            axum::routing::post(endpoints::clone_endpoint),
        )
        .route(
            "/admin/endpoints/delete/{id}",
            axum::routing::post(endpoints::delete_endpoint),
        )
        .route(
            "/admin/endpoints/test",
            axum::routing::post(endpoints::test_endpoint),
        )
        .route(
            "/admin/maintenance",
            axum::routing::get(maintenance::maintenance_page),
        )
        .route(
            "/admin/maintenance/add",
            axum::routing::post(maintenance::add_maintenance),
        )
        .route(
            "/admin/maintenance/edit/{id}",
            axum::routing::post(maintenance::edit_maintenance),
        )
        .route(
            "/admin/maintenance/delete/{id}",
            axum::routing::post(maintenance::delete_maintenance),
        )
        .route(
            "/admin/maintenance/close/{id}",
            axum::routing::post(maintenance::close_maintenance),
        )
        .route(
            "/admin/incidents",
            axum::routing::get(incidents::incidents_page),
        )
        .route(
            "/admin/incidents/create",
            axum::routing::post(incidents::create_incident),
        )
        .route(
            "/admin/incidents/{id}",
            axum::routing::get(incidents::incident_detail),
        )
        .route(
            "/admin/incidents/{id}/update-status",
            axum::routing::post(incidents::update_incident_status),
        )
        .route(
            "/admin/incidents/{id}/comment",
            axum::routing::post(incidents::add_comment),
        )
        .route(
            "/admin/incidents/{id}/public-message",
            axum::routing::post(incidents::update_public_message),
        )
        .route(
            "/admin/incidents/{id}/handover",
            axum::routing::post(incidents::handover_incident),
        )
        .route(
            "/admin/incidents/{id}/postmortem",
            axum::routing::post(incidents::save_postmortem),
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
        .route(
            "/admin/configuration/backup",
            axum::routing::get(configuration::download_backup),
        )
        .route(
            "/admin/configuration/restore",
            axum::routing::post(configuration::restore_backup)
                .layer(DefaultBodyLimit::max(RESTORE_BODY_LIMIT)),
        )
        .route("/admin/users", axum::routing::get(users::users_page))
        .route("/admin/users/add", axum::routing::post(users::add_user))
        .route(
            "/admin/users/password/{id}",
            axum::routing::post(users::change_password),
        )
        .route(
            "/admin/users/email/{id}",
            axum::routing::post(users::update_email),
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
        .route(
            "/admin/reports",
            axum::routing::get(reports::reports_page),
        )
        .route(
            "/admin/reports/preview",
            axum::routing::post(reports::report_preview),
        )
        .route(
            "/admin/reports/schedule",
            axum::routing::post(reports::save_report_schedule),
        )
        .route("/admin/help", axum::routing::get(help::help_page))
        // Embedded static files
        .route("/static/htmx.min.js", axum::routing::get(serve_htmx))
        .route("/static/pico.min.css", axum::routing::get(serve_pico))
        .route("/static/style.css", axum::routing::get(serve_style))
        .route("/static/kernelci-horizontal-color-1.svg", axum::routing::get(serve_logo))
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
