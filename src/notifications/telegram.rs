// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use serde_json::json;

/// Send a message to a Telegram chat via the Bot API.
///
/// `bot_token` is the token issued by @BotFather, `chat_id` is the numeric
/// chat/user/channel id (or `@channelusername`) the bot should post to.
///
/// Error messages never include the token or URL to avoid leaking the secret
/// into logs.
pub async fn send(
    client: &reqwest::Client,
    bot_token: &str,
    chat_id: &str,
    message: &str,
) -> Result<(), String> {
    let url = format!("https://api.telegram.org/bot{bot_token}/sendMessage");

    let body = json!({
        "chat_id": chat_id,
        "text": message,
        "disable_web_page_preview": true,
    });

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|_| "Telegram API request failed".to_string())?;

    if !resp.status().is_success() {
        return Err(format!("Telegram API returned {}", resp.status()));
    }
    Ok(())
}
