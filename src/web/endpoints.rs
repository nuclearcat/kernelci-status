use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect};
use axum::Form;
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::checkers::CheckContext;
use crate::db::endpoints::{Endpoint, NewEndpoint};
use crate::error::AppError;
use crate::state::AppState;

#[derive(Template)]
#[template(path = "endpoints.html")]
struct EndpointsTemplate {
    username: String,
    endpoints: Vec<Endpoint>,
}

pub async fn endpoints_page(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let endpoints = db
        .call(|conn| crate::db::endpoints::list_all(conn))
        .await?;

    Ok(Html(
        EndpointsTemplate {
            username: user.username,
            endpoints,
        }
        .render()
        .unwrap_or_default(),
    ))
}

#[derive(Deserialize)]
pub struct EndpointForm {
    pub name: String,
    pub subname: Option<String>,
    pub endpoint: String,
    pub check_type: String,
    pub selector: Option<String>,
    pub condition: Option<String>,
    pub critical: Option<String>,
    pub enabled: Option<String>,
    pub nodata_behavior: Option<String>,
}

impl EndpointForm {
    fn to_new_endpoint(&self) -> NewEndpoint {
        NewEndpoint {
            name: self.name.clone(),
            subname: self.subname.clone().filter(|s| !s.is_empty()),
            endpoint: self.endpoint.clone(),
            check_type: self.check_type.clone(),
            selector: self.selector.clone().filter(|s| !s.is_empty()),
            condition: self.condition.clone().filter(|s| !s.is_empty()),
            critical: self.critical.as_deref() == Some("on"),
            enabled: self.enabled.as_deref() != Some("off"),
            nodata_behavior: match self.nodata_behavior.as_deref() {
                Some("warning") => "warning".to_string(),
                Some("critical") => "critical".to_string(),
                _ => "nodata".to_string(),
            },
        }
    }
}

pub async fn add_endpoint(
    State(state): State<AppState>,
    _user: AuthUser,
    Form(form): Form<EndpointForm>,
) -> Result<impl IntoResponse, AppError> {
    let ep = form.to_new_endpoint();
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::endpoints::insert(conn, &ep)?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to("/admin/endpoints"))
}

pub async fn edit_endpoint(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<EndpointForm>,
) -> Result<impl IntoResponse, AppError> {
    let ep = form.to_new_endpoint();
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::endpoints::update(conn, id, &ep)?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to("/admin/endpoints"))
}

pub async fn clone_endpoint(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    db.call(move |conn| {
        let ep = crate::db::endpoints::get_by_id(conn, id)?
            .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?;
        let new = NewEndpoint {
            name: ep.name,
            subname: ep.subname,
            endpoint: ep.endpoint,
            check_type: ep.check_type,
            selector: ep.selector,
            condition: ep.condition,
            critical: ep.critical,
            enabled: false,
            nodata_behavior: ep.nodata_behavior,
        };
        crate::db::endpoints::insert(conn, &new)?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to("/admin/endpoints"))
}

pub async fn delete_endpoint(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::endpoints::delete(conn, id)?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to("/admin/endpoints"))
}

pub async fn test_endpoint(
    State(state): State<AppState>,
    _user: AuthUser,
    Form(form): Form<EndpointForm>,
) -> impl IntoResponse {
    let ep = Endpoint {
        id: 0,
        name: form.name.clone(),
        subname: form.subname.clone().filter(|s| !s.is_empty()),
        endpoint: form.endpoint.clone(),
        check_type: form.check_type.clone(),
        selector: form.selector.clone().filter(|s| !s.is_empty()),
        condition: form.condition.clone().filter(|s| !s.is_empty()),
        critical: form.critical.as_deref() == Some("on"),
        enabled: true,
        nodata_behavior: match form.nodata_behavior.as_deref() {
            Some("warning") => "warning".to_string(),
            Some("critical") => "critical".to_string(),
            _ => "nodata".to_string(),
        },
    };

    let ctx = CheckContext {
        http_client: state.http_client.clone(),
    };

    let result = crate::checkers::dispatch_check(&ep, &ctx).await;

    let (badge_class, state_label) = match result.state {
        crate::checkers::EndpointState::Ok => ("badge-ok", "OK"),
        crate::checkers::EndpointState::Warning => ("badge-warning", "WARNING"),
        crate::checkers::EndpointState::Critical => ("badge-critical", "CRITICAL"),
        crate::checkers::EndpointState::NoData => ("badge-nodata", "NO DATA"),
    };

    let value_display = result.value.as_deref().unwrap_or("-");
    let message_display = result.message.as_deref().unwrap_or("-");

    Html(format!(
        r#"<div class="test-result">
            <span class="badge {badge_class}">{state_label}</span>
            <span class="test-detail"><strong>Value:</strong> {value_display}</span>
            <span class="test-detail"><strong>Message:</strong> {message_display}</span>
        </div>"#
    ))
}
