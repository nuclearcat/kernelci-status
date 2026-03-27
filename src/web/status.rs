use askama::Template;
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse};
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::db::history::HistoryEntry;
use crate::error::AppError;
use crate::state::AppState;

/// Number of slots in the timeline bar (constant width regardless of range).
const SLOTS: usize = 96;

#[derive(Deserialize)]
pub struct StatusQuery {
    pub range: Option<String>,
}

/// Parsed time range config.
struct RangeConfig {
    hours: i64,
    slot_minutes: i64,
    label: &'static str,
}

fn parse_range(range: Option<&str>) -> RangeConfig {
    match range {
        Some("7d") => RangeConfig { hours: 168, slot_minutes: 105, label: "7d" },
        Some("30d") => RangeConfig { hours: 720, slot_minutes: 450, label: "30d" },
        _ => RangeConfig { hours: 24, slot_minutes: 15, label: "24h" },
    }
}

/// One slot in the 24h timeline bar.
pub struct TimelineSlot {
    pub css_class: String,
    pub title: String,
}

/// An individual check within a service group.
pub struct CheckStatus {
    pub subname: String,
    pub current_state: String,
    pub current_state_css: String,
}

/// A grouped service row on the public status page.
pub struct ServiceGroup {
    pub name: String,
    pub current_state: String,
    pub current_state_css: String,
    pub uptime_pct: String,
    pub timeline: Vec<TimelineSlot>,
    pub checks: Vec<CheckStatus>,
    pub expandable: bool,
}

/// Shell page — loads instantly, HTMX fetches data.
#[derive(Template)]
#[template(path = "status.html")]
struct StatusShellTemplate {
    year: i32,
}

/// Data fragment returned by /status/data.
#[derive(Template)]
#[template(path = "fragments/status_data.html")]
struct StatusDataTemplate {
    groups: Vec<ServiceGroup>,
    overall_label: String,
    overall_css: String,
    active_range: String,
}

/// Serve the page shell (header + spinner + footer). Data loaded via HTMX.
pub async fn status_page() -> impl IntoResponse {
    Html(
        StatusShellTemplate {
            year: chrono::Utc::now().format("%Y").to_string().parse().unwrap_or(2025),
        }
        .render()
        .unwrap_or_default(),
    )
}

/// Per-endpoint intermediate data before grouping.
struct EndpointData {
    subname: String,
    current_state: String,
    slot_states: Vec<String>,
    /// Uptime from raw entries: (ok_or_warning_count, total_count)
    entry_counts: (usize, usize),
}

/// Serve the status data fragment (called by HTMX on load + every 60s).
pub async fn status_data(
    State(state): State<AppState>,
    Query(query): Query<StatusQuery>,
) -> Result<impl IntoResponse, AppError> {
    let range_cfg = parse_range(query.range.as_deref());
    let hours = range_cfg.hours;
    let slot_minutes = range_cfg.slot_minutes;
    let active_range = range_cfg.label.to_string();

    let db = state.db.clone();
    let groups: Vec<ServiceGroup> = db
        .call(move |conn| {
            let endpoints = crate::db::endpoints::list_all(conn)?;

            // Align `now` to the next slot boundary so that slot edges are
            // stable across consecutive HTMX refreshes.  Without this,
            // each 60-second refresh shifts all 96 boundaries, which can
            // move a lone entry (common right after an outage) into the
            // neighbouring slot, leaving the original slot as NO_DATA.
            let raw_now = chrono::Utc::now();
            let slot_secs = slot_minutes * 60;
            let ts = raw_now.timestamp();
            let remainder = ts % slot_secs;
            let aligned_ts = if remainder == 0 { ts } else { ts + slot_secs - remainder };
            let now = chrono::DateTime::from_timestamp(aligned_ts, 0)
                .unwrap_or(raw_now);

            // Use the same aligned start for the DB query so that the
            // returned entries exactly match the slot-mapping window.
            let start = now - chrono::Duration::hours(hours);
            let start_str = start.format("%Y-%m-%d %H:%M:%S").to_string();

            // Collect per-endpoint data, grouped by name (BTreeMap for stable order).
            let mut by_name: BTreeMap<String, Vec<EndpointData>> = BTreeMap::new();

            for ep in &endpoints {
                if !ep.enabled {
                    continue;
                }
                let entries = crate::db::history::get_since(conn, ep.id, &start_str)?;
                let latest = crate::db::history::get_latest_for_endpoint(conn, ep.id)?;

                let current_state = latest
                    .as_ref()
                    .map(|h| h.state.clone())
                    .unwrap_or_else(|| "NO_DATA".to_string());

                let slot_states = build_slot_states(&entries, now, hours, slot_minutes);

                // Count raw entries for uptime (not slot-based).
                let total = entries.len();
                let ok_count = entries
                    .iter()
                    .filter(|e| e.state == "OK" || e.state == "WARNING")
                    .count();

                by_name
                    .entry(ep.name.clone())
                    .or_default()
                    .push(EndpointData {
                        subname: ep.subname.clone().unwrap_or_default(),
                        current_state,
                        slot_states,
                        entry_counts: (ok_count, total),
                    });
            }

            // Build grouped results.
            let mut result = Vec::new();
            for (name, eps) in &by_name {
                // Merge slot states across all checks: worst per slot (for visualization).
                let mut merged_slots: Vec<&str> = vec!["NO_DATA"; SLOTS];
                for ep in eps {
                    for (i, s) in ep.slot_states.iter().enumerate() {
                        merged_slots[i] = worst_state(merged_slots[i], s);
                    }
                }

                let timeline = slots_to_timeline(&merged_slots, now, hours, slot_minutes);

                // Uptime from raw entries: per-endpoint uptime, then take the minimum.
                // This is independent of slot size, so 24h/7d/30d are consistent.
                let uptime_pct = eps
                    .iter()
                    .filter_map(|e| {
                        if e.entry_counts.1 > 0 {
                            Some(e.entry_counts.0 as f64 / e.entry_counts.1 as f64 * 100.0)
                        } else {
                            None
                        }
                    })
                    .fold(f64::MAX, f64::min);
                let uptime_pct = if uptime_pct == f64::MAX { 100.0 } else { uptime_pct };

                // Group current state = worst across checks.
                let group_state = eps
                    .iter()
                    .map(|e| e.current_state.as_str())
                    .fold("NO_DATA", worst_state);

                let checks: Vec<CheckStatus> = eps
                    .iter()
                    .map(|e| CheckStatus {
                        subname: e.subname.clone(),
                        current_state: e.current_state.clone(),
                        current_state_css: state_to_css(&e.current_state).to_string(),
                    })
                    .collect();

                let expandable = checks.len() > 1;

                result.push(ServiceGroup {
                    name: name.clone(),
                    current_state: group_state.to_string(),
                    current_state_css: state_to_css(group_state).to_string(),
                    uptime_pct: format!("{:.2}%", uptime_pct),
                    timeline,
                    checks,
                    expandable,
                });
            }
            Ok(result)
        })
        .await?;

    let has_critical = groups.iter().any(|g| g.current_state == "CRITICAL");
    let has_warning = groups.iter().any(|g| g.current_state == "WARNING");

    let (overall_label, overall_css) = if has_critical {
        ("Some systems are experiencing issues".to_string(), "overall-critical".to_string())
    } else if has_warning {
        ("Some systems have warnings".to_string(), "overall-warning".to_string())
    } else {
        ("All systems operational".to_string(), "overall-ok".to_string())
    };

    Ok(Html(
        StatusDataTemplate {
            groups,
            overall_label,
            overall_css,
            active_range,
        }
        .render()
        .unwrap_or_default(),
    ))
}

/// Build raw slot states from history entries (used for cross-endpoint merging).
fn build_slot_states(
    entries: &[HistoryEntry],
    now: chrono::DateTime<chrono::Utc>,
    hours: i64,
    slot_minutes: i64,
) -> Vec<String> {
    let start = now - chrono::Duration::hours(hours);
    let mut slot_states: Vec<&str> = vec!["NO_DATA"; SLOTS];

    for entry in entries {
        let ts = match chrono::NaiveDateTime::parse_from_str(&entry.timestamp, "%Y-%m-%d %H:%M:%S")
        {
            Ok(t) => t.and_utc(),
            Err(_) => continue,
        };

        let elapsed = ts - start;
        if elapsed.num_minutes() < 0 {
            continue;
        }
        let slot_idx = (elapsed.num_minutes() / slot_minutes) as usize;
        if slot_idx >= SLOTS {
            continue;
        }

        let current_worst = slot_states[slot_idx];
        slot_states[slot_idx] = worst_state(current_worst, &entry.state);
    }

    slot_states.into_iter().map(|s| s.to_string()).collect()
}

/// Convert slot states into timeline slots (visualization only, no uptime calc).
fn slots_to_timeline(
    slot_states: &[&str],
    now: chrono::DateTime<chrono::Utc>,
    hours: i64,
    slot_minutes: i64,
) -> Vec<TimelineSlot> {
    let start = now - chrono::Duration::hours(hours);

    slot_states
        .iter()
        .enumerate()
        .map(|(i, &state)| {
            let slot_time = start + chrono::Duration::minutes(i as i64 * slot_minutes);
            let time_str = if hours <= 24 {
                slot_time.format("%H:%M").to_string()
            } else {
                slot_time.format("%b %d %H:%M").to_string()
            };
            TimelineSlot {
                css_class: format!("slot-{}", state_to_css(state)),
                title: format!("{} — {}", time_str, state),
            }
        })
        .collect()
}

fn worst_state<'a>(a: &'a str, b: &'a str) -> &'a str {
    let rank = |s: &str| match s {
        "CRITICAL" => 3,
        "WARNING" => 2,
        "OK" => 1,
        _ => 0,
    };
    if rank(b) > rank(a) { b } else { a }
}

fn state_to_css(state: &str) -> &str {
    match state {
        "OK" => "ok",
        "WARNING" => "warning",
        "CRITICAL" => "critical",
        _ => "nodata",
    }
}
