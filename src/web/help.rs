use askama::Template;
use axum::response::{Html, IntoResponse};

use crate::auth::AuthUser;

#[derive(Template)]
#[template(path = "help.html")]
struct HelpTemplate {
    username: String,
}

pub async fn help_page(user: AuthUser) -> impl IntoResponse {
    Html(HelpTemplate { username: user.username }.render().unwrap_or_default())
}
