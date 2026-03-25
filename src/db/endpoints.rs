use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Endpoint {
    pub id: i64,
    pub name: String,
    pub subname: Option<String>,
    pub endpoint: String,
    pub selector: Option<String>,
    pub condition: Option<String>,
    pub critical: bool,
    pub enabled: bool,
}

pub fn list_all(conn: &Connection) -> rusqlite::Result<Vec<Endpoint>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, subname, endpoint, selector, condition, critical, enabled
         FROM endpoints ORDER BY name, subname",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Endpoint {
            id: row.get(0)?,
            name: row.get(1)?,
            subname: row.get(2)?,
            endpoint: row.get(3)?,
            selector: row.get(4)?,
            condition: row.get(5)?,
            critical: row.get(6)?,
            enabled: row.get(7)?,
        })
    })?;
    rows.collect()
}

pub fn get_by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<Endpoint>> {
    conn.query_row(
        "SELECT id, name, subname, endpoint, selector, condition, critical, enabled
         FROM endpoints WHERE id = ?1",
        params![id],
        |row| {
            Ok(Endpoint {
                id: row.get(0)?,
                name: row.get(1)?,
                subname: row.get(2)?,
                endpoint: row.get(3)?,
                selector: row.get(4)?,
                condition: row.get(5)?,
                critical: row.get(6)?,
                enabled: row.get(7)?,
            })
        },
    )
    .optional()
}

pub fn list_enabled(conn: &Connection) -> rusqlite::Result<Vec<Endpoint>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, subname, endpoint, selector, condition, critical, enabled
         FROM endpoints WHERE enabled = 1 ORDER BY name, subname",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Endpoint {
            id: row.get(0)?,
            name: row.get(1)?,
            subname: row.get(2)?,
            endpoint: row.get(3)?,
            selector: row.get(4)?,
            condition: row.get(5)?,
            critical: row.get(6)?,
            enabled: row.get(7)?,
        })
    })?;
    rows.collect()
}

pub struct NewEndpoint {
    pub name: String,
    pub subname: Option<String>,
    pub endpoint: String,
    pub selector: Option<String>,
    pub condition: Option<String>,
    pub critical: bool,
    pub enabled: bool,
}

pub fn insert(conn: &Connection, ep: &NewEndpoint) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO endpoints (name, subname, endpoint, selector, condition, critical, enabled)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            ep.name,
            ep.subname,
            ep.endpoint,
            ep.selector,
            ep.condition,
            ep.critical,
            ep.enabled,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update(conn: &Connection, id: i64, ep: &NewEndpoint) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE endpoints SET name=?1, subname=?2, endpoint=?3, selector=?4,
         condition=?5, critical=?6, enabled=?7 WHERE id=?8",
        params![
            ep.name,
            ep.subname,
            ep.endpoint,
            ep.selector,
            ep.condition,
            ep.critical,
            ep.enabled,
            id,
        ],
    )?;
    Ok(rows > 0)
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
