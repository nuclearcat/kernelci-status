// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use askama::Template;
use axum::Form;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, header::SET_COOKIE};
use axum::response::{Html, IntoResponse, Redirect, Response};
use rand::RngExt;
use serde::Deserialize;
use std::collections::HashMap;

use crate::db::users::User;
use crate::state::AppState;
use crate::web::common::load_config_from_db;

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
    github_enabled: bool,
}

fn session_cookie(token: &str, max_age: u64, secure: bool) -> String {
    // Only set Secure when built-in ACME HTTPS is enabled; plain HTTP deployments need cookies to work.
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure_attr}")
}

fn github_state_cookie(token: &str, max_age: u64, secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!(
        "github_oauth_state={token}; Path=/login/github/callback; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure_attr}"
    )
}

async fn github_enabled(state: &AppState) -> bool {
    load_config_from_db(state).await.is_ok_and(|config| {
        config
            .get("github_client_id")
            .is_some_and(|v| !v.is_empty())
            && config
                .get("github_client_secret")
                .is_some_and(|v| !v.is_empty())
    })
}

async fn render_login(state: &AppState, error: Option<String>) -> Response {
    Html(
        LoginTemplate {
            error,
            github_enabled: github_enabled(state).await,
        }
        .render()
        .unwrap_or_default(),
    )
    .into_response()
}

pub async fn login_page(State(state): State<AppState>) -> Response {
    render_login(&state, None).await
}

#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

pub async fn login_submit(State(state): State<AppState>, Form(form): Form<LoginForm>) -> Response {
    let username = form.username.clone();
    let db = state.db.clone();
    let user = db
        .call(move |conn| crate::db::users::get_by_username(conn, &username))
        .await;

    let user = match user {
        Ok(Some(u)) => u,
        _ => {
            return render_login(&state, Some("Invalid username or password".to_string())).await;
        }
    };

    let valid = crate::auth::password::verify_password(&form.password, &user.password_hash)
        .unwrap_or(false);

    if !valid {
        return render_login(&state, Some("Invalid username or password".to_string())).await;
    }

    match create_session_response(&state, &user).await {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("Failed to create session: {e}");
            render_login(&state, Some("Internal error".to_string())).await
        }
    }
}

async fn create_session_response(state: &AppState, user: &User) -> Result<Response, String> {
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
    db.call(move |conn| crate::db::sessions::create(conn, &tok, uid, &exp))
        .await
        .map_err(|e| e.to_string())?;

    let cookie = session_cookie(&token, 604800, state.secure_cookies);

    // Maintainers can only use the maintenance page; send them straight there.
    let dest = if user.role == "admin" {
        "/admin"
    } else {
        "/admin/maintenance"
    };
    let mut response = Redirect::to(dest).into_response();
    response.headers_mut().append(
        SET_COOKIE,
        cookie.parse::<HeaderValue>().map_err(|e| e.to_string())?,
    );
    Ok(response)
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|c| c.trim().split_once('='))
        .find_map(|(k, v)| (k == name).then(|| v.to_string()))
}

fn oauth_redirect_uri(
    config: &HashMap<String, String>,
    headers: &HeaderMap,
    default_https: bool,
) -> Option<String> {
    if let Some(base_url) = config.get("base_url").filter(|v| !v.is_empty()) {
        return Some(format!(
            "{}/login/github/callback",
            base_url.trim_end_matches('/')
        ));
    }
    let host = headers.get(axum::http::header::HOST)?.to_str().ok()?;
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or(if default_https { "https" } else { "http" });
    Some(format!("{scheme}://{host}/login/github/callback"))
}

pub async fn github_login(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let config = match load_config_from_db(&state).await {
        Ok(config) => config,
        Err(e) => {
            tracing::error!("Failed to load GitHub OAuth config: {e}");
            return render_login(&state, Some("Internal error".to_string())).await;
        }
    };
    let client_id = match config.get("github_client_id").filter(|v| !v.is_empty()) {
        Some(v) => v,
        None => {
            return render_login(&state, Some("GitHub login is not configured".to_string())).await;
        }
    };
    if !config
        .get("github_client_secret")
        .is_some_and(|v| !v.is_empty())
    {
        return render_login(&state, Some("GitHub login is not configured".to_string())).await;
    }
    let redirect_uri = match oauth_redirect_uri(&config, &headers, state.secure_cookies) {
        Some(uri) => uri,
        None => {
            return render_login(
                &state,
                Some("GitHub login requires Public Base URL configuration".to_string()),
            )
            .await;
        }
    };

    let csrf_state: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(48)
        .map(char::from)
        .collect();
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("client_id", client_id);
    query.append_pair("redirect_uri", &redirect_uri);
    query.append_pair("scope", "read:user");
    query.append_pair("state", &csrf_state);
    let url = format!(
        "https://github.com/login/oauth/authorize?{}",
        query.finish()
    );

    let mut response = Redirect::to(&url).into_response();
    response.headers_mut().append(
        SET_COOKIE,
        github_state_cookie(&csrf_state, 600, state.secure_cookies)
            .parse()
            .unwrap(),
    );
    response
}

#[derive(Deserialize)]
pub struct GithubCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct GithubTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct GithubUserResponse {
    login: String,
}

pub async fn github_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GithubCallbackQuery>,
) -> Response {
    if let Some(error) = query.error {
        let msg = query.error_description.unwrap_or(error);
        return login_with_cleared_github_state(&state, msg).await;
    }

    let expected_state = match cookie_value(&headers, "github_oauth_state") {
        Some(v) => v,
        None => {
            return login_with_cleared_github_state(
                &state,
                "GitHub login state expired; try again".to_string(),
            )
            .await;
        }
    };
    if query.state.as_deref() != Some(expected_state.as_str()) {
        return login_with_cleared_github_state(
            &state,
            "GitHub login state mismatch; try again".to_string(),
        )
        .await;
    }
    let code = match query.code {
        Some(code) => code,
        None => {
            return login_with_cleared_github_state(
                &state,
                "GitHub did not return an authorization code".to_string(),
            )
            .await;
        }
    };

    let config = match load_config_from_db(&state).await {
        Ok(config) => config,
        Err(e) => {
            tracing::error!("Failed to load GitHub OAuth config: {e}");
            return login_with_cleared_github_state(&state, "Internal error".to_string()).await;
        }
    };
    let client_id = match config.get("github_client_id").filter(|v| !v.is_empty()) {
        Some(v) => v.clone(),
        None => {
            return login_with_cleared_github_state(
                &state,
                "GitHub login is not configured".to_string(),
            )
            .await;
        }
    };
    let client_secret = match config.get("github_client_secret").filter(|v| !v.is_empty()) {
        Some(v) => v.clone(),
        None => {
            return login_with_cleared_github_state(
                &state,
                "GitHub login is not configured".to_string(),
            )
            .await;
        }
    };
    let redirect_uri = match oauth_redirect_uri(&config, &headers, state.secure_cookies) {
        Some(uri) => uri,
        None => {
            return login_with_cleared_github_state(
                &state,
                "GitHub login requires Public Base URL configuration".to_string(),
            )
            .await;
        }
    };

    let token_body = {
        let mut body = url::form_urlencoded::Serializer::new(String::new());
        body.append_pair("client_id", &client_id);
        body.append_pair("client_secret", &client_secret);
        body.append_pair("code", &code);
        body.append_pair("redirect_uri", &redirect_uri);
        body.finish()
    };

    let token_resp = match state
        .http_client
        .post("https://github.com/login/oauth/access_token")
        .header(axum::http::header::ACCEPT, "application/json")
        .header(
            axum::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(axum::http::header::USER_AGENT, "kernelci-status")
        .body(token_body)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("GitHub token request failed: {e}");
            return login_with_cleared_github_state(
                &state,
                "GitHub login failed during token exchange".to_string(),
            )
            .await;
        }
    };
    let token_body = match token_resp.json::<GithubTokenResponse>().await {
        Ok(body) => body,
        Err(e) => {
            tracing::error!("Failed to parse GitHub token response: {e}");
            return login_with_cleared_github_state(
                &state,
                "GitHub login returned an invalid token response".to_string(),
            )
            .await;
        }
    };
    if let Some(error) = token_body.error {
        let msg = token_body.error_description.unwrap_or(error);
        return login_with_cleared_github_state(&state, msg).await;
    }
    let access_token = match token_body.access_token {
        Some(token) => token,
        None => {
            return login_with_cleared_github_state(
                &state,
                "GitHub login did not return an access token".to_string(),
            )
            .await;
        }
    };

    let github_user = match state
        .http_client
        .get("https://api.github.com/user")
        .header(axum::http::header::USER_AGENT, "kernelci-status")
        .bearer_auth(access_token)
        .send()
        .await
    {
        Ok(resp) => match resp.json::<GithubUserResponse>().await {
            Ok(user) => user,
            Err(e) => {
                tracing::error!("Failed to parse GitHub user response: {e}");
                return login_with_cleared_github_state(
                    &state,
                    "GitHub login returned an invalid user response".to_string(),
                )
                .await;
            }
        },
        Err(e) => {
            tracing::error!("GitHub user request failed: {e}");
            return login_with_cleared_github_state(
                &state,
                "GitHub login failed while fetching the user".to_string(),
            )
            .await;
        }
    };

    let github_login = github_user.login.clone();
    let db = state.db.clone();
    let user = match db
        .call(move |conn| crate::db::users::get_by_github_username(conn, &github_login))
        .await
    {
        Ok(Some(user)) => user,
        Ok(None) => {
            return login_with_cleared_github_state(
                &state,
                format!(
                    "No local user is associated with GitHub username '{}'",
                    github_user.login
                ),
            )
            .await;
        }
        Err(e) => {
            tracing::error!("Failed to look up GitHub user: {e}");
            return login_with_cleared_github_state(&state, "Internal error".to_string()).await;
        }
    };

    match create_session_response(&state, &user).await {
        Ok(mut response) => {
            response.headers_mut().append(
                SET_COOKIE,
                github_state_cookie("", 0, state.secure_cookies)
                    .parse()
                    .unwrap(),
            );
            response
        }
        Err(e) => {
            tracing::error!("Failed to create GitHub login session: {e}");
            login_with_cleared_github_state(&state, "Internal error".to_string()).await
        }
    }
}

async fn login_with_cleared_github_state(state: &AppState, error: String) -> Response {
    let mut response = render_login(state, Some(error)).await;
    response.headers_mut().append(
        SET_COOKIE,
        github_state_cookie("", 0, state.secure_cookies)
            .parse()
            .unwrap(),
    );
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
