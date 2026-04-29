use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize)]
pub struct MaintenanceWindow {
    pub id: i64,
    pub name: String,
    pub start_time: String,
    pub end_time: String,
    pub created_at: String,
    pub endpoint_ids: Vec<i64>,
    pub is_deploy: bool,
    pub changelog: Option<String>,
}

pub struct NewMaintenanceWindow {
    pub name: String,
    pub start_time: String,
    pub end_time: String,
    pub endpoint_ids: Vec<i64>,
    pub is_deploy: bool,
    pub changelog: Option<String>,
}

// TODO: Very old data should be archived, question is how, separate table? flag?
pub fn list_all(conn: &Connection) -> rusqlite::Result<Vec<MaintenanceWindow>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, start_time, end_time, created_at, is_deploy, changelog
         FROM maintenance_windows ORDER BY start_time DESC",
    )?;
    let windows: Vec<MaintenanceWindow> = stmt
        .query_map([], |row| {
            Ok(MaintenanceWindow {
                id: row.get(0)?,
                name: row.get(1)?,
                start_time: row.get(2)?,
                end_time: row.get(3)?,
                created_at: row.get(4)?,
                endpoint_ids: Vec::new(),
                is_deploy: row.get(5)?,
                changelog: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Load endpoint associations
    let mut result = Vec::with_capacity(windows.len());
    for mut w in windows {
        w.endpoint_ids = get_endpoint_ids(conn, w.id)?;
        result.push(w);
    }
    Ok(result)
}

// TODO: Create validation, must not be inserted in past and overlapping with existing windows for same endpoints.
// Probably don't allow to insert maintenance windows on short notice as well, maybe less than 1 hour before start time or something like that. 
pub fn insert(conn: &Connection, mw: &NewMaintenanceWindow) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO maintenance_windows (name, start_time, end_time, is_deploy, changelog) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![mw.name, mw.start_time, mw.end_time, mw.is_deploy, mw.changelog],
    )?;
    let id = conn.last_insert_rowid();
    set_endpoint_ids(conn, id, &mw.endpoint_ids)?;
    Ok(id)
}

// TODO: Also block editing of past maintenance windows for consistency of historical data
// Maybe adding locked flag?
pub fn update(conn: &Connection, id: i64, mw: &NewMaintenanceWindow) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE maintenance_windows SET name=?1, start_time=?2, end_time=?3, is_deploy=?4, changelog=?5 WHERE id=?6",
        params![mw.name, mw.start_time, mw.end_time, mw.is_deploy, mw.changelog, id],
    )?;
    if rows > 0 {
        set_endpoint_ids(conn, id, &mw.endpoint_ids)?;
    }
    Ok(rows > 0)
}

// TODO: We should not delete past maintenance windows for consistency of historical data. 
// Instead we should add an "archived" flag and filter them out of active lists.
// Maybe even don't allow deleting on short notice as well
pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    // Junction table rows deleted by ON DELETE CASCADE
    let rows = conn.execute("DELETE FROM maintenance_windows WHERE id = ?1", params![id])?;
    Ok(rows > 0)
}

/// End an active maintenance window immediately.
pub fn close_early(conn: &Connection, id: i64, ended_at: &str) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE maintenance_windows
         SET end_time = ?1
         WHERE id = ?2 AND start_time <= ?1 AND end_time > ?1",
        params![ended_at, id],
    )?;
    Ok(rows > 0)
}

/// Get all endpoint IDs that are currently in an active maintenance window.
pub fn get_active_endpoint_ids(conn: &Connection, now: &str) -> rusqlite::Result<HashSet<i64>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT mwe.endpoint_id
         FROM maintenance_windows mw
         JOIN maintenance_window_endpoints mwe ON mw.id = mwe.window_id
         WHERE mw.start_time <= ?1 AND mw.end_time > ?1",
    )?;
    let ids = stmt
        .query_map(params![now], |row| row.get::<_, i64>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(ids)
}

/// Get maintenance windows that start within the next `days` days (but haven't started yet).
pub fn get_upcoming(conn: &Connection, now: &str, days: i64) -> rusqlite::Result<Vec<MaintenanceWindow>> {
    let future = chrono::NaiveDateTime::parse_from_str(now, "%Y-%m-%d %H:%M:%S")
        .unwrap_or_default()
        + chrono::Duration::days(days);
    let future_str = future.format("%Y-%m-%d %H:%M:%S").to_string();

    let mut stmt = conn.prepare(
        "SELECT id, name, start_time, end_time, created_at, is_deploy, changelog
         FROM maintenance_windows
         WHERE start_time > ?1 AND start_time <= ?2
         ORDER BY start_time ASC",
    )?;
    let windows: Vec<MaintenanceWindow> = stmt
        .query_map(params![now, future_str], |row| {
            Ok(MaintenanceWindow {
                id: row.get(0)?,
                name: row.get(1)?,
                start_time: row.get(2)?,
                end_time: row.get(3)?,
                created_at: row.get(4)?,
                endpoint_ids: Vec::new(),
                is_deploy: row.get(5)?,
                changelog: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = Vec::with_capacity(windows.len());
    for mut w in windows {
        w.endpoint_ids = get_endpoint_ids(conn, w.id)?;
        result.push(w);
    }
    Ok(result)
}

/// Get maintenance windows that are currently active.
pub fn get_active(conn: &Connection, now: &str) -> rusqlite::Result<Vec<MaintenanceWindow>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, start_time, end_time, created_at, is_deploy, changelog
         FROM maintenance_windows
         WHERE start_time <= ?1 AND end_time > ?1
         ORDER BY start_time ASC",
    )?;
    let windows: Vec<MaintenanceWindow> = stmt
        .query_map(params![now], |row| {
            Ok(MaintenanceWindow {
                id: row.get(0)?,
                name: row.get(1)?,
                start_time: row.get(2)?,
                end_time: row.get(3)?,
                created_at: row.get(4)?,
                endpoint_ids: Vec::new(),
                is_deploy: row.get(5)?,
                changelog: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = Vec::with_capacity(windows.len());
    for mut w in windows {
        w.endpoint_ids = get_endpoint_ids(conn, w.id)?;
        result.push(w);
    }
    Ok(result)
}

/// Get maintenance windows starting within the next hour that haven't had a reminder sent.
pub fn get_needing_reminder(conn: &Connection, now: &str) -> rusqlite::Result<Vec<MaintenanceWindow>> {
    let one_hour = chrono::NaiveDateTime::parse_from_str(now, "%Y-%m-%d %H:%M:%S")
        .unwrap_or_default()
        + chrono::Duration::hours(1);
    let one_hour_str = one_hour.format("%Y-%m-%d %H:%M:%S").to_string();

    let mut stmt = conn.prepare(
        "SELECT id, name, start_time, end_time, created_at, is_deploy, changelog
         FROM maintenance_windows
         WHERE start_time > ?1 AND start_time <= ?2 AND reminder_sent = 0
         ORDER BY start_time ASC",
    )?;
    let windows: Vec<MaintenanceWindow> = stmt
        .query_map(params![now, one_hour_str], |row| {
            Ok(MaintenanceWindow {
                id: row.get(0)?,
                name: row.get(1)?,
                start_time: row.get(2)?,
                end_time: row.get(3)?,
                created_at: row.get(4)?,
                endpoint_ids: Vec::new(),
                is_deploy: row.get(5)?,
                changelog: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = Vec::with_capacity(windows.len());
    for mut w in windows {
        w.endpoint_ids = get_endpoint_ids(conn, w.id)?;
        result.push(w);
    }
    Ok(result)
}

/// Get past deploy maintenance windows (ended within last N days).
pub fn get_past_deploys(conn: &Connection, now: &str, days: i64) -> rusqlite::Result<Vec<MaintenanceWindow>> {
    let past = chrono::NaiveDateTime::parse_from_str(now, "%Y-%m-%d %H:%M:%S")
        .unwrap_or_default()
        - chrono::Duration::days(days);
    let past_str = past.format("%Y-%m-%d %H:%M:%S").to_string();

    let mut stmt = conn.prepare(
        "SELECT id, name, start_time, end_time, created_at, is_deploy, changelog
         FROM maintenance_windows
         WHERE is_deploy = 1 AND end_time <= ?1 AND end_time >= ?2
         ORDER BY end_time DESC",
    )?;
    let windows: Vec<MaintenanceWindow> = stmt
        .query_map(params![now, past_str], |row| {
            Ok(MaintenanceWindow {
                id: row.get(0)?,
                name: row.get(1)?,
                start_time: row.get(2)?,
                end_time: row.get(3)?,
                created_at: row.get(4)?,
                endpoint_ids: Vec::new(),
                is_deploy: row.get(5)?,
                changelog: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut result = Vec::with_capacity(windows.len());
    for mut w in windows {
        w.endpoint_ids = get_endpoint_ids(conn, w.id)?;
        result.push(w);
    }
    Ok(result)
}

pub fn mark_reminder_sent(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE maintenance_windows SET reminder_sent = 1 WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

fn get_endpoint_ids(conn: &Connection, window_id: i64) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT endpoint_id FROM maintenance_window_endpoints WHERE window_id = ?1",
    )?;
    let ids = stmt
        .query_map(params![window_id], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

fn set_endpoint_ids(conn: &Connection, window_id: i64, ids: &[i64]) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM maintenance_window_endpoints WHERE window_id = ?1",
        params![window_id],
    )?;
    for &eid in ids {
        conn.execute(
            "INSERT INTO maintenance_window_endpoints (window_id, endpoint_id) VALUES (?1, ?2)",
            params![window_id, eid],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{close_early, insert, NewMaintenanceWindow};

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE maintenance_windows (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                start_time DATETIME NOT NULL,
                end_time DATETIME NOT NULL,
                created_at DATETIME NOT NULL DEFAULT (datetime('now')),
                reminder_sent BOOLEAN NOT NULL DEFAULT 0,
                is_deploy BOOLEAN NOT NULL DEFAULT 0,
                changelog TEXT
            );
            CREATE TABLE maintenance_window_endpoints (
                window_id INTEGER NOT NULL,
                endpoint_id INTEGER NOT NULL,
                PRIMARY KEY (window_id, endpoint_id)
            );
            ",
        )
        .unwrap();
        conn
    }

    fn window(start_time: &str, end_time: &str) -> NewMaintenanceWindow {
        NewMaintenanceWindow {
            name: "test maintenance".to_string(),
            start_time: start_time.to_string(),
            end_time: end_time.to_string(),
            endpoint_ids: Vec::new(),
            is_deploy: false,
            changelog: None,
        }
    }

    #[test]
    fn close_early_only_updates_active_window() {
        let conn = setup_conn();
        let active_id = insert(
            &conn,
            &window("2026-04-29 10:00:00", "2026-04-29 12:00:00"),
        )
        .unwrap();
        let future_id = insert(
            &conn,
            &window("2026-04-29 13:00:00", "2026-04-29 14:00:00"),
        )
        .unwrap();

        assert!(close_early(&conn, active_id, "2026-04-29 11:00:00").unwrap());
        assert!(!close_early(&conn, future_id, "2026-04-29 11:00:00").unwrap());

        let active_end: String = conn
            .query_row(
                "SELECT end_time FROM maintenance_windows WHERE id = ?1",
                [active_id],
                |row| row.get(0),
            )
            .unwrap();
        let future_end: String = conn
            .query_row(
                "SELECT end_time FROM maintenance_windows WHERE id = ?1",
                [future_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(active_end, "2026-04-29 11:00:00");
        assert_eq!(future_end, "2026-04-29 14:00:00");
    }
}
