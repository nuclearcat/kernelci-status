use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect};
use axum::Form;
use serde::Deserialize;

use crate::auth::AuthUser;
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
    user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let users = db
        .call(|conn| crate::db::users::list_all(conn))
        .await?;

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
}

pub async fn add_user(
    State(state): State<AppState>,
    _user: AuthUser,
    Form(form): Form<AddUserForm>,
) -> Result<impl IntoResponse, AppError> {
    let hash = crate::auth::password::hash_password(&form.password)
        .map_err(|e| AppError::Internal(format!("Password hash error: {e}")))?;

    let username = form.username;
    let db = state.db.clone();
    db.call(move |conn| {
        crate::db::users::insert(conn, &username, &hash)?;
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
    _user: AuthUser,
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

pub async fn delete_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if auth.user_id == id {
        return Err(AppError::BadRequest(
            "Cannot delete your own account".to_string(),
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
