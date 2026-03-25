use crate::checkers::{CheckContext, CheckResult, EndpointState};
use crate::db::endpoints::Endpoint;
use std::time::Duration;
use tokio_postgres::NoTls;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn check(endpoint: &Endpoint, _ctx: &CheckContext) -> CheckResult {
    match do_check(endpoint).await {
        Ok(r) => r,
        Err(e) => CheckResult {
            state: EndpointState::Critical,
            value: None,
            message: Some(e),
        },
    }
}

async fn do_check(endpoint: &Endpoint) -> Result<CheckResult, String> {
    let dsn = &endpoint.endpoint;

    let (client, connection) =
        tokio::time::timeout(CONNECT_TIMEOUT, tokio_postgres::connect(dsn, NoTls))
            .await
            .map_err(|_| "PostgreSQL connection timed out (10s)".to_string())?
            .map_err(|e| format!("PostgreSQL connection failed: {e}"))?;

    // Spawn connection handler
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("PostgreSQL connection error: {e}");
        }
    });

    // If a query is specified in the selector, run it
    if let Some(query) = endpoint.selector.as_deref() {
        if !query.is_empty() {
            let rows = tokio::time::timeout(CONNECT_TIMEOUT, client.query(query, &[]))
                .await
                .map_err(|_| "Query timed out (10s)".to_string())?
                .map_err(|e| format!("Query failed: {e}"))?;

            let value = if let Some(row) = rows.first() {
                row.try_get::<_, String>(0).unwrap_or_default()
            } else {
                String::new()
            };

            return Ok(CheckResult {
                state: EndpointState::Ok,
                value: Some(value),
                message: Some("Query executed successfully".to_string()),
            });
        }
    }

    // Basic up/down check
    tokio::time::timeout(CONNECT_TIMEOUT, client.simple_query("SELECT 1"))
        .await
        .map_err(|_| "Health check timed out (10s)".to_string())?
        .map_err(|e| format!("Health check query failed: {e}"))?;

    Ok(CheckResult {
        state: EndpointState::Ok,
        value: Some("1".to_string()),
        message: Some("PostgreSQL is up".to_string()),
    })
}
