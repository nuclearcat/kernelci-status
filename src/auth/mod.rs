pub mod password;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};

use crate::state::AppState;

/// Authenticated user extracted from session cookie.
pub struct AuthUser {
    pub user_id: i64,
    pub username: String,
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
                            let stored =
                                crate::db::config::get(conn, "api_token")?;
                            Ok(stored.is_some_and(|t| t == api_token))
                        })
                        .await
                        .unwrap_or(false);

                    if valid {
                        return Ok(AuthUser {
                            user_id: 0,
                            username: "api".to_string(),
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
            }),
            _ => Err(Redirect::to("/login").into_response()),
        }
    }
}

/// Extractor for API routes that returns 401 instead of redirect.
#[allow(dead_code)]
pub struct ApiAuth {
    pub user_id: i64,
}

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
                if let Ok(Some(s)) = session {
                    return Ok(ApiAuth {
                        user_id: s.user_id,
                    });
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
                    Ok(stored.is_some_and(|t| t == api_token))
                })
                .await
                .unwrap_or(false);

            if valid {
                return Ok(ApiAuth { user_id: 0 });
            }
        }

        Err(StatusCode::UNAUTHORIZED)
    }
}
