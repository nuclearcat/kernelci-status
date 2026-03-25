use rusqlite::Connection;

pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS endpoints (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            subname TEXT,
            endpoint TEXT NOT NULL,
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
            state TEXT NOT NULL CHECK (state IN ('OK','WARNING','CRITICAL','NO_DATA')),
            message TEXT
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
    )
}
