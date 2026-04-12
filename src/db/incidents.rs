use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Incident {
    pub id: i64,
    pub endpoint_id: i64,
    pub title: String,
    pub severity: String,
    pub status: String,
    pub assigned_user_id: Option<i64>,
    pub public_message: Option<String>,
    pub created_at: String,
    pub acknowledged_at: Option<String>,
    pub resolved_at: Option<String>,
    pub auto_created: bool,
    pub postmortem: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IncidentUpdate {
    pub id: i64,
    pub incident_id: i64,
    pub update_type: String,
    pub status: Option<String>,
    pub message: Option<String>,
    pub user_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct IncidentToken {
    pub incident_id: i64,
    pub user_id: i64,
    pub action: String,
    pub token: String,
}

pub struct NewIncident {
    pub endpoint_id: i64,
    pub title: String,
    pub severity: String,
    pub auto_created: bool,
}

fn row_to_incident(row: &rusqlite::Row) -> rusqlite::Result<Incident> {
    Ok(Incident {
        id: row.get(0)?,
        endpoint_id: row.get(1)?,
        title: row.get(2)?,
        severity: row.get(3)?,
        status: row.get(4)?,
        assigned_user_id: row.get(5)?,
        public_message: row.get(6)?,
        created_at: row.get(7)?,
        acknowledged_at: row.get(8)?,
        resolved_at: row.get(9)?,
        auto_created: row.get(10)?,
        postmortem: row.get(11)?,
    })
}

const INCIDENT_COLS: &str =
    "id, endpoint_id, title, severity, status, assigned_user_id, public_message, \
     created_at, acknowledged_at, resolved_at, auto_created, postmortem";

pub fn insert(conn: &Connection, new: &NewIncident) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO incidents (endpoint_id, title, severity, auto_created) VALUES (?1, ?2, ?3, ?4)",
        params![new.endpoint_id, new.title, new.severity, new.auto_created],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<Incident>> {
    conn.query_row(
        &format!("SELECT {INCIDENT_COLS} FROM incidents WHERE id = ?1"),
        params![id],
        row_to_incident,
    )
    .optional()
}

/// Find an open (not resolved) incident for this endpoint.
pub fn get_open_for_endpoint(conn: &Connection, endpoint_id: i64) -> rusqlite::Result<Option<Incident>> {
    conn.query_row(
        &format!(
            "SELECT {INCIDENT_COLS} FROM incidents \
             WHERE endpoint_id = ?1 AND status != 'resolved' \
             ORDER BY created_at DESC LIMIT 1"
        ),
        params![endpoint_id],
        row_to_incident,
    )
    .optional()
}

pub fn list_all(conn: &Connection) -> rusqlite::Result<Vec<Incident>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {INCIDENT_COLS} FROM incidents \
         ORDER BY CASE WHEN status = 'resolved' THEN 1 ELSE 0 END, created_at DESC"
    ))?;
    let rows = stmt.query_map([], row_to_incident)?;
    rows.collect()
}

/// Return resolved incidents from the last `days` days, most recent first.
pub fn list_recent_resolved(conn: &Connection, days: i64) -> rusqlite::Result<Vec<Incident>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {INCIDENT_COLS} FROM incidents \
         WHERE status = 'resolved' \
         AND resolved_at > datetime('now', '-' || ?1 || ' days') \
         ORDER BY resolved_at DESC"
    ))?;
    let rows = stmt.query_map(params![days], row_to_incident)?;
    rows.collect()
}

pub fn list_active(conn: &Connection) -> rusqlite::Result<Vec<Incident>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {INCIDENT_COLS} FROM incidents \
         WHERE status != 'resolved' ORDER BY created_at DESC"
    ))?;
    let rows = stmt.query_map([], row_to_incident)?;
    rows.collect()
}

pub fn update_status(conn: &Connection, id: i64, status: &str) -> rusqlite::Result<bool> {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let rows = match status {
        "acknowledged" => conn.execute(
            "UPDATE incidents SET status = ?1, acknowledged_at = ?2 WHERE id = ?3",
            params![status, now, id],
        )?,
        "resolved" => conn.execute(
            "UPDATE incidents SET status = ?1, resolved_at = ?2 WHERE id = ?3",
            params![status, now, id],
        )?,
        _ => conn.execute(
            "UPDATE incidents SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?,
    };
    Ok(rows > 0)
}

pub fn assign(conn: &Connection, id: i64, user_id: i64) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE incidents SET assigned_user_id = ?1 WHERE id = ?2",
        params![user_id, id],
    )?;
    Ok(rows > 0)
}

pub fn save_postmortem(conn: &Connection, id: i64, text: &str) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE incidents SET postmortem = ?1 WHERE id = ?2",
        params![text, id],
    )?;
    Ok(rows > 0)
}

pub fn save_public_message(conn: &Connection, id: i64, msg: &str) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE incidents SET public_message = ?1 WHERE id = ?2",
        params![msg, id],
    )?;
    Ok(rows > 0)
}

pub fn add_update(
    conn: &Connection,
    incident_id: i64,
    update_type: &str,
    status: Option<&str>,
    message: Option<&str>,
    user_id: Option<i64>,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO incident_updates (incident_id, update_type, status, message, user_id) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![incident_id, update_type, status, message, user_id],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_updates(conn: &Connection, incident_id: i64) -> rusqlite::Result<Vec<IncidentUpdate>> {
    let mut stmt = conn.prepare(
        "SELECT id, incident_id, update_type, status, message, user_id, created_at \
         FROM incident_updates WHERE incident_id = ?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![incident_id], |row| {
        Ok(IncidentUpdate {
            id: row.get(0)?,
            incident_id: row.get(1)?,
            update_type: row.get(2)?,
            status: row.get(3)?,
            message: row.get(4)?,
            user_id: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    rows.collect()
}

pub fn create_token(
    conn: &Connection,
    incident_id: i64,
    user_id: i64,
    action: &str,
    token: &str,
    expires_at: &str,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO incident_tokens (incident_id, user_id, action, token, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![incident_id, user_id, action, token, expires_at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_valid_token(conn: &Connection, token: &str) -> rusqlite::Result<Option<IncidentToken>> {
    conn.query_row(
        "SELECT incident_id, user_id, action, token \
         FROM incident_tokens \
         WHERE token = ?1 AND used_at IS NULL AND expires_at > datetime('now')",
        params![token],
        |row| {
            Ok(IncidentToken {
                incident_id: row.get(0)?,
                user_id: row.get(1)?,
                action: row.get(2)?,
                token: row.get(3)?,
            })
        },
    )
    .optional()
}

pub fn mark_token_used(conn: &Connection, token: &str) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE incident_tokens SET used_at = datetime('now') WHERE token = ?1",
        params![token],
    )?;
    Ok(rows > 0)
}

/// Get incidents that have been in 'detected' state longer than `minutes` minutes.
pub fn get_unacknowledged_past_threshold(
    conn: &Connection,
    minutes: i64,
) -> rusqlite::Result<Vec<Incident>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {INCIDENT_COLS} FROM incidents \
         WHERE status = 'detected' \
         AND created_at < datetime('now', '-' || ?1 || ' minutes') \
         ORDER BY created_at ASC"
    ))?;
    let rows = stmt.query_map(params![minutes], row_to_incident)?;
    rows.collect()
}
