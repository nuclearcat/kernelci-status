pub mod config;
pub mod endpoints;
pub mod history;
pub mod incidents;
pub mod maintenance;
pub mod reports;
pub mod schema;
pub mod sessions;
pub mod users;

use tokio_rusqlite::Connection;

use crate::error::AppError;

pub async fn open_and_migrate(path: &str) -> Result<Connection, AppError> {
    let conn = Connection::open(path)
        .await
        .map_err(|_| AppError::Internal("Failed to open database".to_string()))?;

    conn.call(|conn| {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        schema::run_migrations(conn)?;
        Ok(())
    })
    .await?;

    Ok(conn)
}
