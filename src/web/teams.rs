// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect};
use axum_extra::extract::Form;
use serde::Deserialize;
use std::collections::BTreeSet;

use crate::auth::AdminUser;
use crate::error::AppError;
use crate::state::AppState;

/// A user shown in a team's member checklist.
struct UserLite {
    id: i64,
    username: String,
    role: String,
}

/// View model for one team: its members and endpoint names, plus the full
/// universe of users/endpoints so the template can render checked checkboxes.
struct TeamView {
    id: i64,
    name: String,
    member_ids: Vec<i64>,
    endpoint_names: Vec<String>,
}

#[derive(Template)]
#[template(path = "teams.html")]
struct TeamsTemplate {
    username: String,
    teams: Vec<TeamView>,
    all_users: Vec<UserLite>,
    all_endpoint_names: Vec<String>,
}

pub async fn teams_page(
    State(state): State<AppState>,
    user: AdminUser,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let (teams, all_users, all_endpoint_names) = db
        .call(|conn| {
            let raw = crate::db::teams::list_all(conn)?;
            let mut teams = Vec::with_capacity(raw.len());
            for t in raw {
                let members = crate::db::teams::members_of(conn, t.id)?;
                let endpoint_names = crate::db::teams::endpoints_of(conn, t.id)?;
                teams.push(TeamView {
                    id: t.id,
                    name: t.name,
                    member_ids: members.into_iter().collect(),
                    endpoint_names,
                });
            }

            let all_users: Vec<UserLite> = crate::db::users::list_all(conn)?
                .into_iter()
                .map(|u| UserLite {
                    id: u.id,
                    username: u.username,
                    role: u.role,
                })
                .collect();

            let names: BTreeSet<String> = crate::db::endpoints::list_all(conn)?
                .into_iter()
                .map(|e| e.name)
                .collect();

            Ok((teams, all_users, names.into_iter().collect::<Vec<_>>()))
        })
        .await?;

    Ok(Html(
        TeamsTemplate {
            username: user.username,
            teams,
            all_users,
            all_endpoint_names,
        }
        .render()
        .unwrap_or_default(),
    ))
}

#[derive(Deserialize)]
pub struct AddTeamForm {
    pub name: String,
}

pub async fn add_team(
    State(state): State<AppState>,
    _user: AdminUser,
    Form(form): Form<AddTeamForm>,
) -> Result<impl IntoResponse, AppError> {
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest(
            "Team name cannot be empty".to_string(),
        ));
    }
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::teams::create(conn, &name)?;
        Ok(())
    })
    .await?;
    Ok(Redirect::to("/admin/teams"))
}

pub async fn delete_team(
    State(state): State<AppState>,
    _user: AdminUser,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::teams::delete(conn, id)?;
        Ok(())
    })
    .await?;
    Ok(Redirect::to("/admin/teams"))
}

#[derive(Deserialize)]
pub struct MembersForm {
    #[serde(default)]
    pub user_ids: Vec<i64>,
}

pub async fn update_members(
    State(state): State<AppState>,
    _user: AdminUser,
    Path(id): Path<i64>,
    Form(form): Form<MembersForm>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::teams::set_members(conn, id, &form.user_ids)?;
        Ok(())
    })
    .await?;
    Ok(Redirect::to("/admin/teams"))
}

#[derive(Deserialize)]
pub struct EndpointsForm {
    #[serde(default)]
    pub endpoint_names: Vec<String>,
}

pub async fn update_endpoints(
    State(state): State<AppState>,
    _user: AdminUser,
    Path(id): Path<i64>,
    Form(form): Form<EndpointsForm>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::teams::set_endpoints(conn, id, &form.endpoint_names)?;
        Ok(())
    })
    .await?;
    Ok(Redirect::to("/admin/teams"))
}
