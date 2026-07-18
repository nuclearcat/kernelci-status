// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

pub mod password;

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};
use subtle::ConstantTimeEq;

use crate::state::AppState;

// Compare API tokens in constant time to avoid leaking token prefix matches through timing.
fn api_token_matches(stored: Option<String>, candidate: &str) -> bool {
    stored.is_some_and(|stored| stored.as_bytes().ct_eq(candidate.as_bytes()).into())
}

/// Authenticated user extracted from session cookie.
pub struct AuthUser {
    pub user_id: i64,
    pub username: String,
    pub role: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let cookie_header = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let token = cookie_header
            .split(';')
            .filter_map(|c| c.trim().strip_prefix("session="))
            .next();

        let token = match token {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => {
                // Check for API token in Authorization header
                let auth_header = parts
                    .headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");

                if let Some(api_token) = auth_header.strip_prefix("Bearer ") {
                    let api_token = api_token.to_string();
                    let db = state.db.clone();
                    let valid: bool = db
                        .call(move |conn| -> rusqlite::Result<bool> {
                            let stored = crate::db::config::get(conn, "api_token")?;
                            Ok(api_token_matches(stored, &api_token))
                        })
                        .await
                        .unwrap_or(false);

                    if valid {
                        return Ok(AuthUser {
                            user_id: 0,
                            username: "api".to_string(),
                            role: "admin".to_string(),
                        });
                    }
                }
                return Err(Redirect::to("/login").into_response());
            }
        };

        let db = state.db.clone();
        let token_clone = token.clone();
        let session = db
            .call(move |conn| crate::db::sessions::get_valid(conn, &token_clone))
            .await;

        let session = match session {
            Ok(Some(s)) => s,
            _ => return Err(Redirect::to("/login").into_response()),
        };

        let user_id = session.user_id;
        let db = state.db.clone();
        let user = db
            .call(move |conn| crate::db::users::get_by_id(conn, user_id))
            .await;

        match user {
            Ok(Some(u)) => Ok(AuthUser {
                user_id: u.id,
                username: u.username,
                role: u.role,
            }),
            _ => Err(Redirect::to("/login").into_response()),
        }
    }
}

/// Authenticated user that must hold the `admin` role. Gates every admin route
/// except maintenance. An authenticated maintainer is redirected to the one page
/// they can use rather than shown a bare 403; an unauthenticated request follows
/// `AuthUser` and is sent to `/login`.
pub struct AdminUser {
    pub user_id: i64,
    pub username: String,
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.role != "admin" {
            return Err(Redirect::to("/admin/maintenance").into_response());
        }
        Ok(AdminUser {
            user_id: user.user_id,
            username: user.username,
        })
    }
}

/// Extractor for API routes that returns 401 instead of redirect.
pub struct ApiAuth;

impl FromRequestParts<AppState> for ApiAuth {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let cookie_header = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let token = cookie_header
            .split(';')
            .filter_map(|c| c.trim().strip_prefix("session="))
            .next();

        if let Some(token) = token {
            if !token.is_empty() {
                let t = token.to_string();
                let db = state.db.clone();
                let session = db
                    .call(move |conn| crate::db::sessions::get_valid(conn, &t))
                    .await;
                if let Ok(Some(_)) = session {
                    return Ok(ApiAuth);
                }
            }
        }

        // Try Bearer token
        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if let Some(api_token) = auth_header.strip_prefix("Bearer ") {
            let api_token = api_token.to_string();
            let db = state.db.clone();
            let valid: bool = db
                .call(move |conn| -> rusqlite::Result<bool> {
                    let stored = crate::db::config::get(conn, "api_token")?;
                    Ok(api_token_matches(stored, &api_token))
                })
                .await
                .unwrap_or(false);

            if valid {
                return Ok(ApiAuth);
            }
        }

        Err(StatusCode::UNAUTHORIZED)
    }
}
