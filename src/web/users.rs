// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect};
use serde::Deserialize;

use crate::auth::AdminUser;
use crate::db::users::User;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Template)]
#[template(path = "users.html")]
struct UsersTemplate {
    username: String,
    users: Vec<User>,
}

pub async fn users_page(
    State(state): State<AppState>,
    user: AdminUser,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let users = db.call(|conn| crate::db::users::list_all(conn)).await?;

    Ok(Html(
        UsersTemplate {
            username: user.username,
            users,
        }
        .render()
        .unwrap_or_default(),
    ))
}

#[derive(Deserialize)]
pub struct AddUserForm {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub role: Option<String>,
}

pub async fn add_user(
    State(state): State<AppState>,
    _user: AdminUser,
    Form(form): Form<AddUserForm>,
) -> Result<impl IntoResponse, AppError> {
    // Default to the least-privileged role if the form omits/garbles it.
    let role = match form.role.as_deref() {
        Some(r) if crate::db::users::valid_role(r) => r.to_string(),
        _ => "maintainer".to_string(),
    };

    let hash = crate::auth::password::hash_password(&form.password)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?;

    let username = form.username;
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::users::insert(conn, &username, &hash, &role)?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to("/admin/users"))
}

#[derive(Deserialize)]
pub struct UpdateRoleForm {
    pub role: String,
}

pub async fn update_role(
    State(state): State<AppState>,
    auth: AdminUser,
    Path(id): Path<i64>,
    Form(form): Form<UpdateRoleForm>,
) -> Result<impl IntoResponse, AppError> {
    if auth.user_id == id {
        return Err(AppError::BadRequest(
            "Cannot change your own role".to_string(),
        ));
    }
    if !crate::db::users::valid_role(&form.role) {
        return Err(AppError::BadRequest("Invalid role".to_string()));
    }

    let db = state.db.clone();
    let (target_role, admin_count) = db
        .call(move |conn| {
            let role = crate::db::users::get_by_id(conn, id)?
                .map(|u| u.role)
                .unwrap_or_default();
            let count = crate::db::users::count_admins(conn)?;
            Ok::<_, rusqlite::Error>((role, count))
        })
        .await?;

    // Don't let the last admin demote themselves out of existence.
    if target_role == "admin" && form.role != "admin" && admin_count <= 1 {
        return Err(AppError::BadRequest(
            "Cannot demote the last admin".to_string(),
        ));
    }

    let role = form.role.clone();
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::users::update_role(conn, id, &role)?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to("/admin/users"))
}

#[derive(Deserialize)]
pub struct ChangePasswordForm {
    pub password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    _user: AdminUser,
    Path(id): Path<i64>,
    Form(form): Form<ChangePasswordForm>,
) -> Result<impl IntoResponse, AppError> {
    if form.password.is_empty() {
        return Err(AppError::BadRequest("Password cannot be empty".to_string()));
    }

    let hash = crate::auth::password::hash_password(&form.password)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?;

    let db = state.db.clone();
    db.call(move |conn| {
        conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            rusqlite::params![hash, id],
        )?;
        conn.execute(
            "DELETE FROM sessions WHERE user_id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to("/admin/users"))
}

#[derive(Deserialize)]
pub struct UpdateEmailForm {
    pub email: String,
}

pub async fn update_email(
    State(state): State<AppState>,
    _user: AdminUser,
    Path(id): Path<i64>,
    Form(form): Form<UpdateEmailForm>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::users::update_email(conn, id, &form.email)?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to("/admin/users"))
}

#[derive(Deserialize)]
pub struct UpdateGithubForm {
    pub github_username: String,
}

pub async fn update_github(
    State(state): State<AppState>,
    _user: AdminUser,
    Path(id): Path<i64>,
    Form(form): Form<UpdateGithubForm>,
) -> Result<impl IntoResponse, AppError> {
    let github_username = form
        .github_username
        .trim()
        .trim_start_matches('@')
        .to_string();
    if !github_username.is_empty() {
        let lookup = github_username.clone();
        let db = state.db.clone();
        if let Some(existing) = db
            .call(move |conn| crate::db::users::get_by_github_username(conn, &lookup))
            .await?
        {
            if existing.id != id {
                return Err(AppError::BadRequest(format!(
                    "GitHub username '{github_username}' is already assigned to another user"
                )));
            }
        }
    }

    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::users::update_github_username(conn, id, &github_username)?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to("/admin/users"))
}

pub async fn delete_user(
    State(state): State<AppState>,
    auth: AdminUser,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if auth.user_id == id {
        return Err(AppError::BadRequest(
            "Cannot delete your own account".to_string(),
        ));
    }

    let db = state.db.clone();
    let (target_role, admin_count) = db
        .call(move |conn| {
            let role = crate::db::users::get_by_id(conn, id)?
                .map(|u| u.role)
                .unwrap_or_default();
            let count = crate::db::users::count_admins(conn)?;
            Ok::<_, rusqlite::Error>((role, count))
        })
        .await?;

    if target_role == "admin" && admin_count <= 1 {
        return Err(AppError::BadRequest(
            "Cannot delete the last admin".to_string(),
        ));
    }

    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::users::delete(conn, id)?;
        Ok(())
    })
    .await?;

    Ok(Redirect::to("/admin/users"))
}
