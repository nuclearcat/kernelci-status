use rusqlite::{params, Connection};

pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS endpoints (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            subname TEXT,
            endpoint TEXT NOT NULL,
            check_type TEXT NOT NULL DEFAULT 'http_status',
            selector TEXT,
            condition TEXT,
            critical BOOLEAN NOT NULL DEFAULT 0,
            enabled BOOLEAN NOT NULL DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS state_history (
            id INTEGER PRIMARY KEY,
            endpoint_id INTEGER NOT NULL REFERENCES endpoints(id),
            timestamp DATETIME NOT NULL DEFAULT (datetime('now')),
            value TEXT,
            state TEXT NOT NULL CHECK (state IN (
                'OK','WARNING','CRITICAL','NO_DATA','MAINTENANCE',
                'OK_MAINTENANCE','WARNING_MAINTENANCE','CRITICAL_MAINTENANCE','NO_DATA_MAINTENANCE'
            )),
            message TEXT
        );

        CREATE TABLE IF NOT EXISTS maintenance_windows (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            start_time DATETIME NOT NULL,
            end_time DATETIME NOT NULL,
            created_at DATETIME NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS maintenance_window_endpoints (
            window_id INTEGER NOT NULL REFERENCES maintenance_windows(id) ON DELETE CASCADE,
            endpoint_id INTEGER NOT NULL REFERENCES endpoints(id) ON DELETE CASCADE,
            PRIMARY KEY (window_id, endpoint_id)
        );

        CREATE TABLE IF NOT EXISTS config (
            key TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            username TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            created_at DATETIME NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS sessions (
            token TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL REFERENCES users(id),
            expires_at DATETIME NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_history_endpoint_ts
            ON state_history(endpoint_id, timestamp DESC);

        CREATE INDEX IF NOT EXISTS idx_sessions_expires
            ON sessions(expires_at);
        ",
    )?;

    migrate_add_check_type(conn)?;
    migrate_maintenance_windows(conn)?;
    migrate_incidents(conn)?;
    migrate_maintenance_reminder(conn)?;

    Ok(())
}

/// Create maintenance_windows tables, update state_history CHECK for compound states,
/// and drop the old `maintenance` boolean from endpoints if present.
fn migrate_maintenance_windows(conn: &Connection) -> rusqlite::Result<()> {
    // Create maintenance tables (IF NOT EXISTS handles fresh DBs)
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS maintenance_windows (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            start_time DATETIME NOT NULL,
            end_time DATETIME NOT NULL,
            created_at DATETIME NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS maintenance_window_endpoints (
            window_id INTEGER NOT NULL REFERENCES maintenance_windows(id) ON DELETE CASCADE,
            endpoint_id INTEGER NOT NULL REFERENCES endpoints(id) ON DELETE CASCADE,
            PRIMARY KEY (window_id, endpoint_id)
        );
        CREATE INDEX IF NOT EXISTS idx_mw_time ON maintenance_windows(start_time, end_time);
        ",
    )?;

    // Drop old maintenance column from endpoints if it exists
    let has_maintenance = {
        let mut stmt = conn.prepare("PRAGMA table_info(endpoints)")?;
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        names.iter().any(|n| n == "maintenance")
    };
    if has_maintenance {
        conn.execute_batch("ALTER TABLE endpoints DROP COLUMN maintenance")?;
    }

    // Update state_history CHECK constraint to support compound states.
    // Probe by trying to insert a compound state.
    let needs_constraint_update = conn
        .execute(
            "INSERT INTO state_history (endpoint_id, state, message) VALUES (-1, 'OK_MAINTENANCE', 'migration_probe')",
            [],
        )
        .is_err();

    if needs_constraint_update {
        conn.execute_batch(
            "
            CREATE TABLE state_history_new (
                id INTEGER PRIMARY KEY,
                endpoint_id INTEGER NOT NULL REFERENCES endpoints(id),
                timestamp DATETIME NOT NULL DEFAULT (datetime('now')),
                value TEXT,
                state TEXT NOT NULL CHECK (state IN (
                    'OK','WARNING','CRITICAL','NO_DATA','MAINTENANCE',
                    'OK_MAINTENANCE','WARNING_MAINTENANCE','CRITICAL_MAINTENANCE','NO_DATA_MAINTENANCE'
                )),
                message TEXT
            );
            INSERT INTO state_history_new SELECT * FROM state_history;
            DROP TABLE state_history;
            ALTER TABLE state_history_new RENAME TO state_history;
            CREATE INDEX IF NOT EXISTS idx_history_endpoint_ts
                ON state_history(endpoint_id, timestamp DESC);
            ",
        )?;
    } else {
        conn.execute(
            "DELETE FROM state_history WHERE endpoint_id = -1 AND message = 'migration_probe'",
            [],
        )?;
    }

    Ok(())
}

/// Migrate existing endpoints to use the new check_type column.
/// Runs once: detects if the column is missing and adds it with data migration.
fn migrate_add_check_type(conn: &Connection) -> rusqlite::Result<()> {
    let has_check_type = {
        let mut stmt = conn.prepare("PRAGMA table_info(endpoints)")?;
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        names.iter().any(|n| n == "check_type")
    };

    if has_check_type {
        return Ok(());
    }

    conn.execute_batch(
        "ALTER TABLE endpoints ADD COLUMN check_type TEXT NOT NULL DEFAULT 'http_status'",
    )?;

    // Prometheus: rewrite promhttps:// and promhttp:// to real URLs
    conn.execute_batch(
        "
        UPDATE endpoints SET check_type = 'prometheus',
            endpoint = REPLACE(REPLACE(endpoint, 'promhttps://', 'https://'), 'promhttp://', 'http://')
        WHERE endpoint LIKE 'promhttp://%' OR endpoint LIKE 'promhttps://%';

        UPDATE endpoints SET endpoint = endpoint || '/metrics'
        WHERE check_type = 'prometheus' AND endpoint NOT LIKE '%/metrics';
        ",
    )?;

    // TLS cert: selector was 'cert_expiration'
    conn.execute_batch(
        "
        UPDATE endpoints SET check_type = 'tls_cert', selector = NULL
        WHERE (endpoint LIKE 'http://%' OR endpoint LIKE 'https://%')
          AND selector = 'cert_expiration';
        ",
    )?;

    // HTTP latency: selector was 'latency'
    conn.execute_batch(
        "
        UPDATE endpoints SET check_type = 'http_latency', selector = NULL
        WHERE (endpoint LIKE 'http://%' OR endpoint LIKE 'https://%')
          AND selector = 'latency';
        ",
    )?;

    // PostgreSQL
    conn.execute_batch(
        "UPDATE endpoints SET check_type = 'postgresql' WHERE endpoint LIKE 'postgresql://%';",
    )?;

    // Docker via SSH
    conn.execute_batch(
        "UPDATE endpoints SET check_type = 'docker' WHERE endpoint LIKE 'ssh://%';",
    )?;

    // Kubernetes: parse k8s://namespace/labels → endpoint=namespace, selector=labels
    {
        let mut stmt =
            conn.prepare("SELECT id, endpoint FROM endpoints WHERE endpoint LIKE 'k8s://%'")?;
        let k8s_rows: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;

        for (id, url) in k8s_rows {
            let path = url.strip_prefix("k8s://").unwrap_or(&url);
            let (namespace, labels) = match path.split_once('/') {
                Some((ns, sel)) => (ns.to_string(), Some(sel.to_string())),
                None => (path.to_string(), None),
            };
            conn.execute(
                "UPDATE endpoints SET check_type = 'kubernetes', endpoint = ?1, selector = ?2 WHERE id = ?3",
                params![namespace, labels, id],
            )?;
        }
    }

    Ok(())
}

fn migrate_incidents(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS incidents (
            id INTEGER PRIMARY KEY,
            endpoint_id INTEGER NOT NULL REFERENCES endpoints(id),
            title TEXT NOT NULL,
            severity TEXT NOT NULL DEFAULT 'critical',
            status TEXT NOT NULL DEFAULT 'detected',
            assigned_user_id INTEGER REFERENCES users(id),
            public_message TEXT,
            created_at DATETIME NOT NULL DEFAULT (datetime('now')),
            acknowledged_at DATETIME,
            resolved_at DATETIME,
            auto_created INTEGER NOT NULL DEFAULT 0,
            postmortem TEXT
        );

        CREATE TABLE IF NOT EXISTS incident_updates (
            id INTEGER PRIMARY KEY,
            incident_id INTEGER NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
            update_type TEXT NOT NULL,
            status TEXT,
            message TEXT,
            user_id INTEGER,
            created_at DATETIME NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS incident_tokens (
            id INTEGER PRIMARY KEY,
            incident_id INTEGER NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
            user_id INTEGER NOT NULL,
            action TEXT NOT NULL,
            token TEXT NOT NULL UNIQUE,
            used_at DATETIME,
            expires_at DATETIME NOT NULL,
            created_at DATETIME NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_incidents_endpoint_status
            ON incidents(endpoint_id, status);
        CREATE INDEX IF NOT EXISTS idx_incident_tokens_token
            ON incident_tokens(token);
        CREATE INDEX IF NOT EXISTS idx_incident_updates_incident
            ON incident_updates(incident_id, created_at);
        ",
    )?;

    // Add email column to users table if it doesn't exist
    let has_email = {
        let mut stmt = conn.prepare("PRAGMA table_info(users)")?;
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        names.iter().any(|n| n == "email")
    };
    if !has_email {
        conn.execute_batch("ALTER TABLE users ADD COLUMN email TEXT")?;
    }

    Ok(())
}

fn migrate_maintenance_reminder(conn: &Connection) -> rusqlite::Result<()> {
    let has_col = {
        let mut stmt = conn.prepare("PRAGMA table_info(maintenance_windows)")?;
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        names.iter().any(|n| n == "reminder_sent")
    };
    if !has_col {
        conn.execute_batch(
            "ALTER TABLE maintenance_windows ADD COLUMN reminder_sent BOOLEAN NOT NULL DEFAULT 0",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    /// Simulate the OLD schema (without check_type), insert rows, run migration, verify.
    #[test]
    fn test_migrate_add_check_type() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();

        // Create old schema WITHOUT check_type
        conn.execute_batch(
            "
            CREATE TABLE endpoints (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                subname TEXT,
                endpoint TEXT NOT NULL,
                selector TEXT,
                condition TEXT,
                critical BOOLEAN NOT NULL DEFAULT 0,
                enabled BOOLEAN NOT NULL DEFAULT 1
            );
            ",
        )
        .unwrap();

        // Insert test rows with old-style data
        conn.execute_batch(
            "
            INSERT INTO endpoints (name, endpoint, selector) VALUES ('api', 'https://api.example.com', NULL);
            INSERT INTO endpoints (name, endpoint, selector) VALUES ('api-lat', 'https://api.example.com', 'latency');
            INSERT INTO endpoints (name, endpoint, selector) VALUES ('api-cert', 'https://api.example.com', 'cert_expiration');
            INSERT INTO endpoints (name, endpoint, selector) VALUES ('prom', 'promhttps://metrics.example.com', '(http_requests_total,status=500)');
            INSERT INTO endpoints (name, endpoint, selector) VALUES ('prom-plain', 'promhttp://metrics.example.com', NULL);
            INSERT INTO endpoints (name, endpoint, selector) VALUES ('prom-metrics', 'promhttps://metrics.example.com/metrics', '(up)');
            INSERT INTO endpoints (name, endpoint, selector) VALUES ('pg', 'postgresql://user:pass@db/app', 'SELECT 1');
            INSERT INTO endpoints (name, endpoint, selector) VALUES ('k8s-ns', 'k8s://production', NULL);
            INSERT INTO endpoints (name, endpoint, selector) VALUES ('k8s-label', 'k8s://production/app=api', NULL);
            INSERT INTO endpoints (name, endpoint, selector) VALUES ('docker', 'ssh://deploy@host', 'nginx');
            ",
        )
        .unwrap();

        // Run migration
        super::migrate_add_check_type(&conn).unwrap();

        // Verify results
        let mut stmt = conn
            .prepare("SELECT name, endpoint, check_type, selector FROM endpoints ORDER BY id")
            .unwrap();
        let rows: Vec<(String, String, String, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        // http_status: plain https, no magic selector
        assert_eq!(rows[0], ("api".into(), "https://api.example.com".into(), "http_status".into(), None));
        // http_latency: selector was 'latency', now cleared
        assert_eq!(rows[1], ("api-lat".into(), "https://api.example.com".into(), "http_latency".into(), None));
        // tls_cert: selector was 'cert_expiration', now cleared
        assert_eq!(rows[2], ("api-cert".into(), "https://api.example.com".into(), "tls_cert".into(), None));
        // prometheus: promhttps rewritten, /metrics appended
        assert_eq!(rows[3], ("prom".into(), "https://metrics.example.com/metrics".into(), "prometheus".into(), Some("(http_requests_total,status=500)".into())));
        // prometheus: promhttp rewritten, /metrics appended
        assert_eq!(rows[4], ("prom-plain".into(), "http://metrics.example.com/metrics".into(), "prometheus".into(), None));
        // prometheus: already had /metrics, no duplication
        assert_eq!(rows[5], ("prom-metrics".into(), "https://metrics.example.com/metrics".into(), "prometheus".into(), Some("(up)".into())));
        // postgresql: unchanged
        assert_eq!(rows[6], ("pg".into(), "postgresql://user:pass@db/app".into(), "postgresql".into(), Some("SELECT 1".into())));
        // kubernetes: namespace only
        assert_eq!(rows[7], ("k8s-ns".into(), "production".into(), "kubernetes".into(), None));
        // kubernetes: namespace + labels split
        assert_eq!(rows[8], ("k8s-label".into(), "production".into(), "kubernetes".into(), Some("app=api".into())));
        // docker: unchanged
        assert_eq!(rows[9], ("docker".into(), "ssh://deploy@host".into(), "docker".into(), Some("nginx".into())));

        // Running migration again should be a no-op (column already exists)
        super::migrate_add_check_type(&conn).unwrap();
    }
}
