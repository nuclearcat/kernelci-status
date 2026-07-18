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
    pub role: String,
    pub github_username: Option<String>,
}

/// Roles a user may hold. `admin` has full access; `maintainer` may only manage
/// maintenance windows for endpoints within their team scope.
pub fn valid_role(role: &str) -> bool {
    matches!(role, "admin" | "maintainer")
}

pub fn get_by_username(conn: &Connection, username: &str) -> rusqlite::Result<Option<User>> {
    conn.query_row(
        "SELECT id, username, password_hash, created_at, email, role, github_username FROM users WHERE username = ?1",
        params![username],
        |row| {
            Ok(User {
                id: row.get(0)?,
                username: row.get(1)?,
                password_hash: row.get(2)?,
                created_at: row.get(3)?,
                email: row.get(4)?,
                role: row.get(5)?,
                github_username: row.get(6)?,
            })
        },
    )
    .optional()
}

pub fn get_by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<User>> {
    conn.query_row(
        "SELECT id, username, password_hash, created_at, email, role, github_username FROM users WHERE id = ?1",
        params![id],
        |row| {
            Ok(User {
                id: row.get(0)?,
                username: row.get(1)?,
                password_hash: row.get(2)?,
                created_at: row.get(3)?,
                email: row.get(4)?,
                role: row.get(5)?,
                github_username: row.get(6)?,
            })
        },
    )
    .optional()
}

pub fn get_by_github_username(
    conn: &Connection,
    github_username: &str,
) -> rusqlite::Result<Option<User>> {
    conn.query_row(
        "SELECT id, username, password_hash, created_at, email, role, github_username \
         FROM users WHERE github_username = ?1 COLLATE NOCASE",
        params![github_username],
        |row| {
            Ok(User {
                id: row.get(0)?,
                username: row.get(1)?,
                password_hash: row.get(2)?,
                created_at: row.get(3)?,
                email: row.get(4)?,
                role: row.get(5)?,
                github_username: row.get(6)?,
            })
        },
    )
    .optional()
}

pub fn list_all(conn: &Connection) -> rusqlite::Result<Vec<User>> {
    let mut stmt = conn.prepare(
        "SELECT id, username, password_hash, created_at, email, role, github_username FROM users ORDER BY username",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(User {
            id: row.get(0)?,
            username: row.get(1)?,
            password_hash: row.get(2)?,
            created_at: row.get(3)?,
            email: row.get(4)?,
            role: row.get(5)?,
            github_username: row.get(6)?,
        })
    })?;
    rows.collect()
}

pub fn insert(
    conn: &Connection,
    username: &str,
    password_hash: &str,
    role: &str,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO users (username, password_hash, role) VALUES (?1, ?2, ?3)",
        params![username, password_hash, role],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_role(conn: &Connection, id: i64, role: &str) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE users SET role = ?1 WHERE id = ?2",
        params![role, id],
    )?;
    Ok(rows > 0)
}

/// Number of users with the `admin` role — used to prevent locking out the last
/// administrator.
pub fn count_admins(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM users WHERE role = 'admin'", [], |r| {
        r.get(0)
    })
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

pub fn update_github_username(
    conn: &Connection,
    id: i64,
    github_username: &str,
) -> rusqlite::Result<bool> {
    let clean = github_username.trim().trim_start_matches('@');
    let val = if clean.is_empty() { None } else { Some(clean) };
    let rows = conn.execute(
        "UPDATE users SET github_username = ?1 WHERE id = ?2",
        params![val, id],
    )?;
    Ok(rows > 0)
}

/// Return all users that have a non-null, non-empty email.
pub fn list_with_email(conn: &Connection) -> rusqlite::Result<Vec<User>> {
    let mut stmt = conn.prepare(
        "SELECT id, username, password_hash, created_at, email, role, github_username \
         FROM users WHERE email IS NOT NULL AND email != '' ORDER BY username",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(User {
            id: row.get(0)?,
            username: row.get(1)?,
            password_hash: row.get(2)?,
            created_at: row.get(3)?,
            email: row.get(4)?,
            role: row.get(5)?,
            github_username: row.get(6)?,
        })
    })?;
    rows.collect()
}

pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    // Delete associated sessions and team memberships first
    conn.execute("DELETE FROM sessions WHERE user_id = ?1", params![id])?;
    conn.execute("DELETE FROM team_members WHERE user_id = ?1", params![id])?;
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
        let user_id = insert(&conn, "admin", "old-hash", "admin").unwrap();
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
