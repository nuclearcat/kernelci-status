// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub created_at: String,
    pub email: Option<String>,
}

pub fn get_by_username(conn: &Connection, username: &str) -> rusqlite::Result<Option<User>> {
    conn.query_row(
        "SELECT id, username, password_hash, created_at, email FROM users WHERE username = ?1",
        params![username],
        |row| {
            Ok(User {
                id: row.get(0)?,
                username: row.get(1)?,
                password_hash: row.get(2)?,
                created_at: row.get(3)?,
                email: row.get(4)?,
            })
        },
    )
    .optional()
}

pub fn get_by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<User>> {
    conn.query_row(
        "SELECT id, username, password_hash, created_at, email FROM users WHERE id = ?1",
        params![id],
        |row| {
            Ok(User {
                id: row.get(0)?,
                username: row.get(1)?,
                password_hash: row.get(2)?,
                created_at: row.get(3)?,
                email: row.get(4)?,
            })
        },
    )
    .optional()
}

pub fn list_all(conn: &Connection) -> rusqlite::Result<Vec<User>> {
    let mut stmt = conn.prepare(
        "SELECT id, username, password_hash, created_at, email FROM users ORDER BY username",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(User {
            id: row.get(0)?,
            username: row.get(1)?,
            password_hash: row.get(2)?,
            created_at: row.get(3)?,
            email: row.get(4)?,
        })
    })?;
    rows.collect()
}

pub fn insert(conn: &Connection, username: &str, password_hash: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO users (username, password_hash) VALUES (?1, ?2)",
        params![username, password_hash],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_email(conn: &Connection, id: i64, email: &str) -> rusqlite::Result<bool> {
    let val = if email.trim().is_empty() {
        None
    } else {
        Some(email.trim())
    };
    let rows = conn.execute(
        "UPDATE users SET email = ?1 WHERE id = ?2",
        params![val, id],
    )?;
    Ok(rows > 0)
}

/// Return all users that have a non-null, non-empty email.
pub fn list_with_email(conn: &Connection) -> rusqlite::Result<Vec<User>> {
    let mut stmt = conn.prepare(
        "SELECT id, username, password_hash, created_at, email \
         FROM users WHERE email IS NOT NULL AND email != '' ORDER BY username",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(User {
            id: row.get(0)?,
            username: row.get(1)?,
            password_hash: row.get(2)?,
            created_at: row.get(3)?,
            email: row.get(4)?,
        })
    })?;
    rows.collect()
}

pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    // Delete associated sessions first
    conn.execute("DELETE FROM sessions WHERE user_id = ?1", params![id])?;
    let rows = conn.execute("DELETE FROM users WHERE id = ?1", params![id])?;
    Ok(rows > 0)
}
