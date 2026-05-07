// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::db::history::HistoryEntry;

#[derive(Debug, Clone, Serialize)]
pub struct Endpoint {
    pub id: i64,
    pub name: String,
    pub subname: Option<String>,
    pub endpoint: String,
    pub check_type: String,
    pub selector: Option<String>,
    pub condition: Option<String>,
    pub critical: bool,
    pub enabled: bool,
    pub nodata_behavior: String,
}

#[derive(Debug, Clone)]
pub struct EndpointWithState {
    pub endpoint: Endpoint,
    pub state: String,
    pub value: Option<String>,
    pub message: Option<String>,
    pub last_check: Option<String>,
}

impl EndpointWithState {
    pub fn new(endpoint: Endpoint, latest: Option<&HistoryEntry>) -> Self {
        Self {
            state: latest
                .map(|h| h.state.clone())
                .unwrap_or_else(|| "NO_DATA".to_string()),
            value: latest.and_then(|h| h.value.clone()),
            message: latest.and_then(|h| h.message.clone()),
            last_check: latest.map(|h| h.timestamp.clone()),
            endpoint,
        }
    }
}

pub fn list_all(conn: &Connection) -> rusqlite::Result<Vec<Endpoint>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, subname, endpoint, check_type, selector, condition, critical, enabled, nodata_behavior
         FROM endpoints ORDER BY name, subname",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Endpoint {
            id: row.get(0)?,
            name: row.get(1)?,
            subname: row.get(2)?,
            endpoint: row.get(3)?,
            check_type: row.get(4)?,
            selector: row.get(5)?,
            condition: row.get(6)?,
            critical: row.get(7)?,
            enabled: row.get(8)?,
            nodata_behavior: row.get(9)?,
        })
    })?;
    rows.collect()
}

pub fn list_all_with_latest_state(conn: &Connection) -> rusqlite::Result<Vec<EndpointWithState>> {
    let endpoints = list_all(conn)?;
    let latest_by_endpoint = crate::db::history::get_latest_by_endpoint(conn)?;
    Ok(endpoints
        .into_iter()
        .map(|endpoint| {
            let latest = latest_by_endpoint.get(&endpoint.id);
            EndpointWithState::new(endpoint, latest)
        })
        .collect())
}

pub fn get_by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<Endpoint>> {
    conn.query_row(
        "SELECT id, name, subname, endpoint, check_type, selector, condition, critical, enabled, nodata_behavior
         FROM endpoints WHERE id = ?1",
        params![id],
        |row| {
            Ok(Endpoint {
                id: row.get(0)?,
                name: row.get(1)?,
                subname: row.get(2)?,
                endpoint: row.get(3)?,
                check_type: row.get(4)?,
                selector: row.get(5)?,
                condition: row.get(6)?,
                critical: row.get(7)?,
                enabled: row.get(8)?,
                nodata_behavior: row.get(9)?,
            })
        },
    )
    .optional()
}

pub fn list_enabled(conn: &Connection) -> rusqlite::Result<Vec<Endpoint>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, subname, endpoint, check_type, selector, condition, critical, enabled, nodata_behavior
         FROM endpoints WHERE enabled = 1 ORDER BY name, subname",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Endpoint {
            id: row.get(0)?,
            name: row.get(1)?,
            subname: row.get(2)?,
            endpoint: row.get(3)?,
            check_type: row.get(4)?,
            selector: row.get(5)?,
            condition: row.get(6)?,
            critical: row.get(7)?,
            enabled: row.get(8)?,
            nodata_behavior: row.get(9)?,
        })
    })?;
    rows.collect()
}

pub struct NewEndpoint {
    pub name: String,
    pub subname: Option<String>,
    pub endpoint: String,
    pub check_type: String,
    pub selector: Option<String>,
    pub condition: Option<String>,
    pub critical: bool,
    pub enabled: bool,
    pub nodata_behavior: String,
}

pub fn insert(conn: &Connection, ep: &NewEndpoint) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO endpoints (name, subname, endpoint, check_type, selector, condition, critical, enabled, nodata_behavior)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            ep.name,
            ep.subname,
            ep.endpoint,
            ep.check_type,
            ep.selector,
            ep.condition,
            ep.critical,
            ep.enabled,
            ep.nodata_behavior,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, ep: &NewEndpoint) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE endpoints SET name=?1, subname=?2, endpoint=?3, check_type=?4,
         selector=?5, condition=?6, critical=?7, enabled=?8, nodata_behavior=?9 WHERE id=?10",
        params![
            ep.name,
            ep.subname,
            ep.endpoint,
            ep.check_type,
            ep.selector,
            ep.condition,
            ep.critical,
            ep.enabled,
            ep.nodata_behavior,
            id,
        ],
    )?;
    Ok(rows > 0)
}

pub fn get_ids_by_names(conn: &Connection, names: &[String]) -> rusqlite::Result<Vec<i64>> {
    let mut ids = Vec::new();
    let mut stmt = conn.prepare("SELECT id FROM endpoints WHERE name = ?1")?;
    for name in names {
        let row_ids = stmt
            .query_map(params![name], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.extend(row_ids);
    }
    Ok(ids)
}

pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    // Delete associated history first
    conn.execute(
        "DELETE FROM state_history WHERE endpoint_id = ?1",
        params![id],
    )?;
    let rows = conn.execute("DELETE FROM endpoints WHERE id = ?1", params![id])?;
    Ok(rows > 0)
}
