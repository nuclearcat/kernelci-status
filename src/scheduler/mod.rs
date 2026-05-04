pub mod runner;

use std::time::Duration;
use tokio::sync::watch;
use tracing::{error, info};

use crate::state::AppState;

/// Default check interval if not configured.
const DEFAULT_INTERVAL_MINS: u64 = 5;

pub async fn run(state: AppState, mut shutdown_rx: watch::Receiver<bool>) {
    let initial_delay = compute_initial_delay(&state).await;

    match initial_delay {
        Duration::ZERO => {
            info!("No recent checks found, running immediately");
        }
        d => {
            info!("Last check was recent, next check in {}s", d.as_secs());
            tokio::select! {
                _ = tokio::time::sleep(d) => {}
                _ = shutdown_rx.changed() => {
                    info!("Scheduler shutting down before first check");
                    return;
                }
            }
        }
    }

    // Run first check cycle
    info!("Running scheduled checks");
    if let Err(e) = runner::run_all_checks(&state).await {
        error!("Check cycle failed: {e}");
    }
    cleanup_expired_sessions(&state).await;
    crate::web::incidents::check_escalations(&state).await;
    crate::web::maintenance::check_maintenance_reminders(&state).await;

    let interval_mins = get_interval_mins(&state).await;
    info!("Scheduler running ({interval_mins} minute interval)");

    loop {
        let interval_mins = get_interval_mins(&state).await;
        let sleep = tokio::time::sleep(Duration::from_secs(interval_mins * 60));

        tokio::select! {
            _ = sleep => {
                info!("Running scheduled checks");
                if let Err(e) = runner::run_all_checks(&state).await {
                    error!("Check cycle failed: {e}");
                }
                cleanup_expired_sessions(&state).await;
                crate::web::incidents::check_escalations(&state).await;
                crate::web::maintenance::check_maintenance_reminders(&state).await;
            }
            _ = shutdown_rx.changed() => {
                info!("Scheduler shutting down");
                break;
            }
        }
    }
}

async fn cleanup_expired_sessions(state: &AppState) {
    let db = state.db.clone();
    match db
        .call(|conn| crate::db::sessions::delete_expired(conn))
        .await
    {
        Ok(deleted) if deleted > 0 => info!("Cleaned up {deleted} expired sessions"),
        Ok(_) => {}
        Err(e) => error!("Failed to clean up expired sessions: {e}"),
    }
}

/// Read check interval from config cache. Falls back to DEFAULT_INTERVAL_MINS.
async fn get_interval_mins(state: &AppState) -> u64 {
    let cache = state.config_cache.read().await;
    cache
        .get("check_interval")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&v| v >= 1 && v <= 1440)
        .unwrap_or(DEFAULT_INTERVAL_MINS)
}

/// Look at the most recent check in state_history and compute how long
/// to wait before the next cycle. If the last check was longer ago than
/// the configured interval (or there are no checks), returns zero.
async fn compute_initial_delay(state: &AppState) -> Duration {
    let interval_mins = get_interval_mins(state).await;
    let interval_secs = interval_mins * 60;

    let db = state.db.clone();
    let last_ts = db
        .call(|conn| crate::db::history::get_last_check_timestamp(conn))
        .await
        .unwrap_or(None);

    let last_ts = match last_ts {
        Some(ts) => ts,
        None => return Duration::ZERO,
    };

    let last_time = match chrono::NaiveDateTime::parse_from_str(&last_ts, "%Y-%m-%d %H:%M:%S") {
        Ok(t) => t.and_utc(),
        Err(_) => return Duration::ZERO,
    };

    let now = chrono::Utc::now();
    let elapsed = now - last_time;
    let interval = chrono::Duration::seconds(interval_secs as i64);

    if elapsed >= interval {
        Duration::ZERO
    } else {
        let remaining = interval - elapsed;
        Duration::from_secs(remaining.num_seconds().max(0) as u64)
    }
}
