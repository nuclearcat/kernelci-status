// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

//! Teams group maintainers with the endpoints they're allowed to schedule
//! maintenance for. A maintainer's scope = the union of `team_endpoints` across
//! every team they belong to. Endpoints are referenced by name (the unit the
//! maintenance picker uses), so renaming an endpoint orphans the scope until an
//! admin re-assigns it.

use rusqlite::{Connection, params};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct Team {
    pub id: i64,
    pub name: String,
}

pub fn list_all(conn: &Connection) -> rusqlite::Result<Vec<Team>> {
    let mut stmt = conn.prepare("SELECT id, name FROM teams ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        Ok(Team {
            id: row.get(0)?,
            name: row.get(1)?,
        })
    })?;
    rows.collect()
}

pub fn create(conn: &Connection, name: &str) -> rusqlite::Result<i64> {
    conn.execute("INSERT INTO teams (name) VALUES (?1)", params![name])?;
    Ok(conn.last_insert_rowid())
}

pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    // Explicit cleanup mirrors users::delete; ON DELETE CASCADE also covers this.
    conn.execute("DELETE FROM team_members WHERE team_id = ?1", params![id])?;
    conn.execute("DELETE FROM team_endpoints WHERE team_id = ?1", params![id])?;
    let rows = conn.execute("DELETE FROM teams WHERE id = ?1", params![id])?;
    Ok(rows > 0)
}

pub fn members_of(conn: &Connection, team_id: i64) -> rusqlite::Result<HashSet<i64>> {
    let mut stmt = conn.prepare("SELECT user_id FROM team_members WHERE team_id = ?1")?;
    stmt.query_map(params![team_id], |row| row.get::<_, i64>(0))?
        .collect()
}

pub fn endpoints_of(conn: &Connection, team_id: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT endpoint_name FROM team_endpoints WHERE team_id = ?1 ORDER BY endpoint_name",
    )?;
    stmt.query_map(params![team_id], |row| row.get::<_, String>(0))?
        .collect()
}

pub fn set_members(conn: &Connection, team_id: i64, user_ids: &[i64]) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM team_members WHERE team_id = ?1",
        params![team_id],
    )?;
    for uid in user_ids {
        conn.execute(
            "INSERT OR IGNORE INTO team_members (team_id, user_id) VALUES (?1, ?2)",
            params![team_id, uid],
        )?;
    }
    Ok(())
}

pub fn set_endpoints(conn: &Connection, team_id: i64, names: &[String]) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM team_endpoints WHERE team_id = ?1",
        params![team_id],
    )?;
    for name in names {
        conn.execute(
            "INSERT OR IGNORE INTO team_endpoints (team_id, endpoint_name) VALUES (?1, ?2)",
            params![team_id, name],
        )?;
    }
    Ok(())
}

/// The set of endpoint names a user may manage maintenance for, across all their
/// teams. Empty for a user on no team.
pub fn allowed_endpoint_names_for_user(
    conn: &Connection,
    user_id: i64,
) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT te.endpoint_name
         FROM team_endpoints te
         JOIN team_members tm ON tm.team_id = te.team_id
         WHERE tm.user_id = ?1",
    )?;
    stmt.query_map(params![user_id], |row| row.get::<_, String>(0))?
        .collect()
}
