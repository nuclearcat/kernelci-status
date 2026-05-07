// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use std::collections::HashMap;

use crate::error::{AppError, DbError};
use crate::state::AppState;

pub fn is_valid_email(email: &str) -> bool {
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    if parts.len() != 2 {
        return false;
    }
    let local = parts[0];
    let domain = parts[1];
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    if !domain.contains('.') {
        return false;
    }
    !email.contains(' ')
}

pub async fn load_config(state: &AppState) -> Result<HashMap<String, String>, AppError> {
    load_config_from_db(state).await.map_err(|e| e.into())
}

pub async fn load_config_from_db(state: &AppState) -> Result<HashMap<String, String>, DbError> {
    let db = state.db.clone();
    db.call(|conn| {
        let pairs = crate::db::config::get_all(conn)?;
        Ok(pairs.into_iter().collect())
    })
    .await
}
