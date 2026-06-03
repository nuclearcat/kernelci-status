// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use askama::Template;
use axum::response::{Html, IntoResponse};

use crate::auth::AdminUser;

#[derive(Template)]
#[template(path = "help.html")]
struct HelpTemplate {
    username: String,
}

pub async fn help_page(user: AdminUser) -> impl IntoResponse {
    Html(
        HelpTemplate {
            username: user.username,
        }
        .render()
        .unwrap_or_default(),
    )
}
