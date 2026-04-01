use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub endpoint_id: i64,
    pub timestamp: String,
    pub value: Option<String>,
    pub state: String,
    pub message: Option<String>,
}

pub fn insert(
    conn: &Connection,
    endpoint_id: i64,
    value: Option<&str>,
    state: &str,
    message: Option<&str>,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO state_history (endpoint_id, value, state, message)
         VALUES (?1, ?2, ?3, ?4)",
        params![endpoint_id, value, state, message],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_latest_for_endpoint(
    conn: &Connection,
    endpoint_id: i64,
) -> rusqlite::Result<Option<HistoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, endpoint_id, timestamp, value, state, message
         FROM state_history WHERE endpoint_id = ?1
         ORDER BY timestamp DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map(params![endpoint_id], map_row)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

pub fn get_for_endpoint(
    conn: &Connection,
    endpoint_id: i64,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<Vec<HistoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, endpoint_id, timestamp, value, state, message
         FROM state_history WHERE endpoint_id = ?1
         ORDER BY timestamp DESC LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt.query_map(params![endpoint_id, limit, offset], map_row)?;
    rows.collect()
}

pub fn get_all(
    conn: &Connection,
    limit: i64,
    offset: i64,
    endpoint_filter: Option<i64>,
    state_filter: Option<&str>,
) -> rusqlite::Result<Vec<HistoryEntry>> {
    let mut sql = String::from(
        "SELECT id, endpoint_id, timestamp, value, state, message FROM state_history WHERE 1=1",
    );
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(eid) = endpoint_filter {
        sql.push_str(" AND endpoint_id = ?");
        param_values.push(Box::new(eid));
    }
    if let Some(st) = state_filter {
        sql.push_str(" AND state = ?");
        param_values.push(Box::new(st.to_string()));
    }
    sql.push_str(" ORDER BY timestamp DESC LIMIT ? OFFSET ?");
    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

    let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params.as_slice(), map_row)?;
    rows.collect()
}

/// Get the value recorded approximately `hours_ago` hours ago for a given endpoint.
/// Uses a ±30 minute tolerance window.
pub fn get_value_hours_ago(
    conn: &Connection,
    endpoint_id: i64,
    hours_ago: f64,
) -> rusqlite::Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT value FROM state_history
         WHERE endpoint_id = ?1
           AND timestamp BETWEEN datetime('now', ?2)
                              AND datetime('now', ?3)
         ORDER BY timestamp DESC LIMIT 1",
    )?;
    let lower = format!("-{} minutes", (hours_ago * 60.0 + 30.0) as i64);
    let upper = format!("-{} minutes", ((hours_ago * 60.0 - 30.0).max(0.0)) as i64);
    let mut rows = stmt.query_map(params![endpoint_id, lower, upper], |row| row.get(0))?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Count entries older than `months` months.
pub fn count_old_entries(conn: &Connection, months: i64) -> rusqlite::Result<i64> {
    let cutoff = format!("-{months} months");
    conn.query_row(
        "SELECT COUNT(*) FROM state_history WHERE timestamp < datetime('now', ?1)",
        params![cutoff],
        |row| row.get(0),
    )
}

/// Export a batch of entries older than `months` months to SQL format.
/// Returns the SQL text for one batch. Use with increasing `offset` until empty.
pub fn export_old_entries_batch(
    conn: &Connection,
    months: i64,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<String> {
    let cutoff = format!("-{months} months");
    let mut stmt = conn.prepare(
        "SELECT endpoint_id, timestamp, value, state, message
         FROM state_history
         WHERE timestamp < datetime('now', ?1)
         ORDER BY timestamp
         LIMIT ?2 OFFSET ?3",
    )?;

    let mut buf = String::new();
    let rows = stmt.query_map(params![cutoff, limit, offset], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    for row in rows {
        let (endpoint_id, timestamp, value, state, message) = row?;
        let val_sql = quote_sql_nullable(value.as_deref());
        let msg_sql = quote_sql_nullable(message.as_deref());
        let ts_sql = quote_sql_literal(&timestamp);
        let state_sql = quote_sql_literal(&state);
        buf.push_str(&format!(
            "INSERT INTO state_history (endpoint_id, timestamp, value, state, message) VALUES ({endpoint_id}, {ts_sql}, {val_sql}, {state_sql}, {msg_sql});\n"
        ));
    }
    Ok(buf)
}

/// Properly escape a string for SQL literal: wrap in quotes, double any internal quotes.
fn quote_sql_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn quote_sql_nullable(s: Option<&str>) -> String {
    match s {
        Some(v) => quote_sql_literal(v),
        None => "NULL".to_string(),
    }
}

/// Delete entries older than `months` months.
pub fn delete_old_entries(conn: &Connection, months: i64) -> rusqlite::Result<usize> {
    let cutoff = format!("-{months} months");
    conn.execute(
        "DELETE FROM state_history WHERE timestamp < datetime('now', ?1)",
        params![cutoff],
    )
}

/// Count total entries for pagination.
pub fn count_all(
    conn: &Connection,
    endpoint_filter: Option<i64>,
    state_filter: Option<&str>,
) -> rusqlite::Result<i64> {
    let mut sql = String::from("SELECT COUNT(*) FROM state_history WHERE 1=1");
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(eid) = endpoint_filter {
        sql.push_str(" AND endpoint_id = ?");
        param_values.push(Box::new(eid));
    }
    if let Some(st) = state_filter {
        sql.push_str(" AND state = ?");
        param_values.push(Box::new(st.to_string()));
    }

    let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    conn.query_row(&sql, params.as_slice(), |row| row.get(0))
}

/// Get the timestamp of the most recent check across all endpoints.
pub fn get_last_check_timestamp(conn: &Connection) -> rusqlite::Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT timestamp FROM state_history ORDER BY timestamp DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map([], |row| row.get(0))?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

/// Get all history entries for an endpoint from the last N hours.
#[allow(dead_code)]
pub fn get_last_hours(
    conn: &Connection,
    endpoint_id: i64,
    hours: i64,
) -> rusqlite::Result<Vec<HistoryEntry>> {
    let offset = format!("-{hours} hours");
    let mut stmt = conn.prepare(
        "SELECT id, endpoint_id, timestamp, value, state, message
         FROM state_history
         WHERE endpoint_id = ?1 AND timestamp >= datetime('now', ?2)
         ORDER BY timestamp ASC",
    )?;
    let rows = stmt.query_map(params![endpoint_id, offset], map_row)?;
    rows.collect()
}

/// Get all history entries for an endpoint since a given timestamp.
/// The caller supplies the exact cutoff so that the DB query and the
/// slot-mapping logic share the same time reference (prevents stale
/// NO_DATA slots during HTMX refreshes).
pub fn get_since(
    conn: &Connection,
    endpoint_id: i64,
    since: &str,
) -> rusqlite::Result<Vec<HistoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, endpoint_id, timestamp, value, state, message
         FROM state_history
         WHERE endpoint_id = ?1 AND timestamp >= ?2
         ORDER BY timestamp ASC",
    )?;
    let rows = stmt.query_map(params![endpoint_id, since], map_row)?;
    rows.collect()
}

/// A resolved outage period extracted from state_history transitions.
#[derive(Debug, Clone)]
pub struct OutagePeriod {
    pub endpoint_id: i64,
    pub start_time: String,
    pub end_time: Option<String>,
}

/// Find resolved outage periods (CRITICAL → non-CRITICAL transitions) in the last N days.
/// Only returns outages that have ended (i.e. endpoint recovered). Uses SQL window functions.
pub fn list_outage_periods(conn: &Connection, days: i64) -> rusqlite::Result<Vec<OutagePeriod>> {
    let cutoff = format!("-{days} days");
    let mut stmt = conn.prepare(
        "WITH ranked AS (
            SELECT endpoint_id, timestamp, state,
                   LAG(state) OVER (PARTITION BY endpoint_id ORDER BY timestamp) AS prev_state
            FROM state_history
            WHERE timestamp > datetime('now', ?1)
        ),
        starts AS (
            SELECT endpoint_id, timestamp AS start_time
            FROM ranked
            WHERE state IN ('CRITICAL', 'CRITICAL_MAINTENANCE')
              AND (prev_state IS NULL OR prev_state NOT IN ('CRITICAL', 'CRITICAL_MAINTENANCE'))
        ),
        ends AS (
            SELECT endpoint_id, timestamp AS end_time
            FROM ranked
            WHERE state NOT IN ('CRITICAL', 'CRITICAL_MAINTENANCE')
              AND prev_state IN ('CRITICAL', 'CRITICAL_MAINTENANCE')
        )
        SELECT s.endpoint_id, s.start_time,
               (SELECT MIN(e.end_time) FROM ends e
                WHERE e.endpoint_id = s.endpoint_id AND e.end_time > s.start_time) AS end_time
        FROM starts s
        ORDER BY s.start_time DESC",
    )?;
    let rows = stmt.query_map(params![cutoff], |row| {
        Ok(OutagePeriod {
            endpoint_id: row.get(0)?,
            start_time: row.get(1)?,
            end_time: row.get(2)?,
        })
    })?;
    // Only return resolved outages (end_time is not null)
    let all: Vec<OutagePeriod> = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(all.into_iter().filter(|o| o.end_time.is_some()).collect())
}

fn map_row(row: &rusqlite::Row) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get(0)?,
        endpoint_id: row.get(1)?,
        timestamp: row.get(2)?,
        value: row.get(3)?,
        state: row.get(4)?,
        message: row.get(5)?,
    })
}
