use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect};
use axum::Form;
use serde::Deserialize;

use crate::auth::AuthUser;
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
    pub selector: Option<String>,
    pub condition: Option<String>,
    pub critical: Option<String>,
    pub enabled: Option<String>,
}

impl EndpointForm {
    fn to_new_endpoint(&self) -> NewEndpoint {
        NewEndpoint {
            name: self.name.clone(),
            subname: self.subname.clone().filter(|s| !s.is_empty()),
            endpoint: self.endpoint.clone(),
            selector: self.selector.clone().filter(|s| !s.is_empty()),
            condition: self.condition.clone().filter(|s| !s.is_empty()),
            critical: self.critical.as_deref() == Some("on"),
            enabled: self.enabled.as_deref() != Some("off"),
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
