use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{error, info};
use rusqlite;

use crate::checkers::{self, CheckContext, CheckResult, EndpointState};
use crate::db::endpoints::Endpoint;
use crate::notifications::NotificationEvent;
use crate::state::AppState;

/// Maximum number of checks running at the same time.
const MAX_CONCURRENT_CHECKS: usize = 20;

/// Per-check timeout.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// Default retries on failure (Critical / NoData).
const DEFAULT_RETRIES: u32 = 3;

/// Default retries on warning.
const DEFAULT_WARNING_RETRIES: u32 = 3;

/// Delay between retries.
const RETRY_DELAY: Duration = Duration::from_secs(5);

pub async fn run_all_checks(state: &AppState) -> Result<(), String> {
    let db = state.db.clone();
    let endpoints: Vec<Endpoint> = db
        .call(|conn| crate::db::endpoints::list_enabled(conn))
        .await
        .map_err(|e| format!("Failed to load endpoints: {e}"))?;

    if endpoints.is_empty() {
        info!("No enabled endpoints to check");
        return Ok(());
    }

    let (max_retries, max_warning_retries) = {
        let cache = state.config_cache.read().await;
        let r = cache
            .get("check_retries")
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|&v| v <= 10)
            .unwrap_or(DEFAULT_RETRIES);
        let w = cache
            .get("warning_retries")
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|&v| v <= 10)
            .unwrap_or(DEFAULT_WARNING_RETRIES);
        (r, w)
    };

    info!(
        "Checking {} endpoints (retries: {max_retries}, warning retries: {max_warning_retries})",
        endpoints.len()
    );

    // Fetch endpoints currently in active maintenance windows (batch query)
    let maintenance_ids: HashSet<i64> = {
        let db = state.db.clone();
        db.call(|conn| {
            let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
            crate::db::maintenance::get_active_endpoint_ids(conn, &now)
        })
        .await
        .unwrap_or_default()
    };

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_CHECKS));
    let mut join_set = JoinSet::new();

    for endpoint in endpoints {
        let ctx_client = state.http_client.clone();
        let ep = endpoint.clone();
        let sem = semaphore.clone();
        join_set.spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            let ctx = CheckContext {
                http_client: ctx_client,
            };

            let mut result = run_check_with_timeout(&ep, &ctx).await;

            // Retry on non-OK state (separate retry counts for warnings vs critical)
            let retries_for_state = match result.state {
                EndpointState::Ok => 0,
                EndpointState::Warning => max_warning_retries,
                _ => max_retries,
            };
            if retries_for_state > 0 {
                for attempt in 1..=retries_for_state {
                    tokio::time::sleep(RETRY_DELAY).await;
                    let retry = run_check_with_timeout(&ep, &ctx).await;
                    if retry.state == EndpointState::Ok {
                        info!(
                            "Endpoint {} recovered on retry {}/{}",
                            ep.name, attempt, retries_for_state
                        );
                        result = retry;
                        break;
                    }
                    result = retry;
                }
            }

            (ep, result)
        });
    }

    while let Some(join_result) = join_set.join_next().await {
        let (endpoint, mut check_result) = match join_result {
            Ok(r) => r,
            Err(e) => {
                error!("Check task panicked: {e}");
                continue;
            }
        };

        // Apply condition (can only make state worse)
        if let Some(condition) = &endpoint.condition {
            if !condition.is_empty() {
                let past_value = if checkers::condition::parse_diff_hours(condition).is_some() {
                    let hours =
                        checkers::condition::parse_diff_hours(condition).unwrap_or(12.0);
                    let eid = endpoint.id;
                    let db = state.db.clone();
                    db.call(move |conn| {
                        crate::db::history::get_value_hours_ago(conn, eid, hours)
                    })
                    .await
                    .unwrap_or(None)
                } else {
                    None
                };

                let condition_state = checkers::condition::evaluate(
                    condition,
                    check_result.value.as_deref(),
                    past_value.as_deref(),
                );

                if condition_state > check_result.state {
                    check_result.state = condition_state;
                }
            }
        }

        // Get previous state for transition detection
        let eid = endpoint.id;
        let db = state.db.clone();
        let prev_state: Option<String> = db
            .call(move |conn| -> rusqlite::Result<Option<String>> {
                let h = crate::db::history::get_latest_for_endpoint(conn, eid)?;
                Ok(h.map(|h| h.state))
            })
            .await
            .unwrap_or(None);

        // Store result — append _MAINTENANCE if endpoint is in an active window
        let in_maintenance = maintenance_ids.contains(&endpoint.id);
        let state_str = if in_maintenance {
            format!("{}_MAINTENANCE", check_result.state)
        } else {
            check_result.state.to_string()
        };
        let value = check_result.value.clone();
        let message = check_result.message.clone();
        let eid = endpoint.id;
        let db = state.db.clone();
        let ss = state_str.clone();
        if let Err(e) = db
            .call(move |conn| -> rusqlite::Result<()> {
                crate::db::history::insert(
                    conn,
                    eid,
                    value.as_deref(),
                    &ss,
                    message.as_deref(),
                )?;
                Ok(())
            })
            .await
        {
            error!("Failed to store check result for {}: {e}", endpoint.name);
        }

        // Detect state transition and emit notification
        let prev = prev_state.as_deref().unwrap_or("NO_DATA");
        if prev != state_str {
            info!(
                "State change: {} ({}) {} → {}",
                endpoint.name,
                endpoint.subname.as_deref().unwrap_or(""),
                prev,
                state_str
            );

            // Suppress notifications during maintenance, but notify when
            // maintenance ends and the state is still bad
            let new_is_maint = state_str.ends_with("_MAINTENANCE");
            let prev_is_maint = prev.ends_with("_MAINTENANCE");
            let should_notify = if new_is_maint {
                // Entering or staying in maintenance — suppress
                false
            } else if prev_is_maint {
                // Maintenance just ended — always notify (real state exposed)
                true
            } else {
                // Normal transition — notify
                true
            };

            if should_notify {
                let _ = state.notify_tx.try_send(NotificationEvent {
                    endpoint_name: endpoint.name.clone(),
                    subname: endpoint.subname.clone(),
                    old_state: prev.to_string(),
                    new_state: state_str.clone(),
                    message: check_result.message,
                    value: check_result.value,
                    critical: endpoint.critical,
                });
            }

            // ── Incident auto-management ──
            let base = state_str
                .strip_suffix("_MAINTENANCE")
                .unwrap_or(&state_str);

            if base == "CRITICAL" && !new_is_maint {
                // Auto-create incident if none open for this endpoint
                let eid = endpoint.id;
                let db = state.db.clone();
                let has_open = db
                    .call(move |conn| -> rusqlite::Result<bool> {
                        Ok(crate::db::incidents::get_open_for_endpoint(conn, eid)?.is_some())
                    })
                    .await
                    .unwrap_or(false);

                if !has_open {
                    let ep_name = match &endpoint.subname {
                        Some(sub) => format!("{} ({})", endpoint.name, sub),
                        None => endpoint.name.clone(),
                    };
                    let title = format!("{} is CRITICAL", ep_name);
                    let eid = endpoint.id;
                    let t = title.clone();
                    let db = state.db.clone();
                    let inc_id = db
                        .call(move |conn| -> rusqlite::Result<i64> {
                            let new = crate::db::incidents::NewIncident {
                                endpoint_id: eid,
                                title: t,
                                severity: "critical".to_string(),
                                auto_created: true,
                            };
                            let id = crate::db::incidents::insert(conn, &new)?;
                            crate::db::incidents::add_update(
                                conn,
                                id,
                                "status_change",
                                Some("detected"),
                                Some("Auto-detected by monitoring"),
                                None,
                            )?;
                            Ok(id)
                        })
                        .await;

                    if let Ok(inc_id) = inc_id {
                        info!("Auto-created incident #{inc_id} for {}", endpoint.name);
                        crate::web::incidents::send_incident_created_emails(
                            state,
                            inc_id,
                            &title,
                            &ep_name,
                            "critical",
                        )
                        .await;
                    }
                }
            } else if base == "OK" {
                // Auto-resolve open auto-created incidents for this endpoint
                let eid = endpoint.id;
                let db = state.db.clone();
                let _ = db
                    .call(move |conn| -> rusqlite::Result<()> {
                        if let Some(inc) =
                            crate::db::incidents::get_open_for_endpoint(conn, eid)?
                        {
                            if inc.auto_created {
                                crate::db::incidents::update_status(conn, inc.id, "resolved")?;
                                crate::db::incidents::add_update(
                                    conn,
                                    inc.id,
                                    "auto_resolve",
                                    Some("resolved"),
                                    Some("Endpoint returned to OK"),
                                    None,
                                )?;
                                tracing::info!("Auto-resolved incident #{}", inc.id);
                            }
                        }
                        Ok(())
                    })
                    .await;
            }
        }
    }

    info!("Check cycle complete");
    Ok(())
}

async fn run_check_with_timeout(ep: &Endpoint, ctx: &CheckContext) -> CheckResult {
    match tokio::time::timeout(CHECK_TIMEOUT, checkers::dispatch_check(ep, ctx)).await {
        Ok(r) => r,
        Err(_) => CheckResult {
            state: EndpointState::Critical,
            value: None,
            message: Some("Check timed out (30s)".to_string()),
        },
    }
}
