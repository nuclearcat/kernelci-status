// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use rusqlite::{Connection, OptionalExtension, params};

pub struct Session {
    pub user_id: i64,
}

pub fn create(
    conn: &Connection,
    token: &str,
    user_id: i64,
    expires_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sessions (token, user_id, expires_at) VALUES (?1, ?2, ?3)",
        params![token, user_id, expires_at],
    )?;
    Ok(())
}

pub fn get_valid(conn: &Connection, token: &str) -> rusqlite::Result<Option<Session>> {
    conn.query_row(
        "SELECT user_id FROM sessions
         WHERE token = ?1 AND expires_at > datetime('now')",
        params![token],
        |row| {
            Ok(Session {
                user_id: row.get(0)?,
            })
        },
    )
    .optional()
}

pub fn delete(conn: &Connection, token: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM sessions WHERE token = ?1", params![token])?;
    Ok(())
}

pub fn delete_expired(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM sessions WHERE expires_at <= datetime('now')",
        [],
    )
}
