use askama::Template;
use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Form;
use rand::RngExt;
use serde::Deserialize;

use crate::state::AppState;

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
}

fn session_cookie(token: &str, max_age: u64, secure: bool) -> String {
    // Only set Secure when built-in ACME HTTPS is enabled; plain HTTP deployments need cookies to work.
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure_attr}")
}

pub async fn login_page() -> impl IntoResponse {
    Html(
        LoginTemplate { error: None }
            .render()
            .unwrap_or_default(),
    )
}

#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

pub async fn login_submit(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    let username = form.username.clone();
    let db = state.db.clone();
    let user = db
        .call(move |conn| crate::db::users::get_by_username(conn, &username))
        .await;

    let user = match user {
        Ok(Some(u)) => u,
        _ => {
            return Html(
                LoginTemplate {
                    error: Some("Invalid username or password".to_string()),
                }
                .render()
                .unwrap_or_default(),
            )
            .into_response();
        }
    };

    let valid = crate::auth::password::verify_password(&form.password, &user.password_hash)
        .unwrap_or(false);

    if !valid {
        return Html(
            LoginTemplate {
                error: Some("Invalid username or password".to_string()),
            }
            .render()
            .unwrap_or_default(),
        )
        .into_response();
    }

    // Generate session token
    let token: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect();

    let expires_at = (chrono::Utc::now() + chrono::Duration::days(7))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    let tok = token.clone();
    let uid = user.id;
    let exp = expires_at.clone();
    let db = state.db.clone();
    if let Err(e) = db
        .call(move |conn| crate::db::sessions::create(conn, &tok, uid, &exp))
        .await
    {
        tracing::error!("Failed to create session: {e}");
        return Html(
            LoginTemplate {
                error: Some("Internal error".to_string()),
            }
            .render()
            .unwrap_or_default(),
        )
        .into_response();
    }

    let cookie = session_cookie(&token, 604800, state.secure_cookies);

    let mut response = Redirect::to("/admin").into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, cookie.parse().unwrap());
    response
}

pub async fn logout(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Response {
    if let Some(cookie) = headers.get(axum::http::header::COOKIE) {
        if let Ok(cookie_str) = cookie.to_str() {
            if let Some(token) = cookie_str
                .split(';')
                .filter_map(|c| c.trim().strip_prefix("session="))
                .next()
            {
                let token = token.to_string();
                let db = state.db.clone();
                let _ = db
                    .call(move |conn| crate::db::sessions::delete(conn, &token))
                    .await;
            }
        }
    }

    let cookie = session_cookie("", 0, state.secure_cookies);
    let mut response = Redirect::to("/login").into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, cookie.parse().unwrap());
    response
}
