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

/// Update a user's password and invalidate all of their active sessions.
/// Returns false when the username does not exist.
pub fn update_password_by_username(
    conn: &mut Connection,
    username: &str,
    password_hash: &str,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    let rows = tx.execute(
        "UPDATE users SET password_hash = ?1 WHERE username = ?2",
        params![password_hash, username],
    )?;

    if rows == 0 {
        return Ok(false);
    }

    tx.execute(
        "DELETE FROM sessions WHERE user_id = (SELECT id FROM users WHERE username = ?1)",
        params![username],
    )?;
    tx.commit()?;
    Ok(true)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_update_revokes_sessions() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::schema::run_migrations(&conn).unwrap();
        let user_id = insert(&conn, "admin", "old-hash").unwrap();
        conn.execute(
            "INSERT INTO sessions (token, user_id, expires_at) VALUES (?1, ?2, ?3)",
            params!["token", user_id, "2099-01-01 00:00:00"],
        )
        .unwrap();

        assert!(update_password_by_username(&mut conn, "admin", "new-hash").unwrap());
        assert_eq!(
            get_by_username(&conn, "admin")
                .unwrap()
                .unwrap()
                .password_hash,
            "new-hash"
        );
        let session_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(session_count, 0);
    }

    #[test]
    fn password_update_reports_missing_user() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::schema::run_migrations(&conn).unwrap();

        assert!(!update_password_by_username(&mut conn, "missing", "new-hash").unwrap());
    }
}
