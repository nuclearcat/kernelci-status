use askama::Template;
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse};
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::db::history::HistoryEntry;
use crate::db::maintenance::MaintenanceWindow;
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
    pub sla_uptime_pct: String,
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

/// A maintenance window with resolved endpoint names for display.
pub struct MaintenanceBanner {
    pub name: String,
    pub start_time: String,
    pub end_time: String,
    pub endpoint_names: Vec<String>,
}

/// Data fragment returned by /status/data.
#[derive(Template)]
#[template(path = "fragments/status_data.html")]
struct StatusDataTemplate {
    groups: Vec<ServiceGroup>,
    overall_label: String,
    overall_css: String,
    active_range: String,
    active_maintenance: Vec<MaintenanceBanner>,
    upcoming_maintenance: Vec<MaintenanceBanner>,
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
    /// Absolute uptime: (ok_or_warning_count, total_count) — ignores _MAINTENANCE tag
    entry_counts: (usize, usize),
    /// SLA uptime: (ok_or_warning_count, non_maintenance_total) — excludes *_MAINTENANCE entirely
    sla_entry_counts: (usize, usize),
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
    let (groups, active_maintenance, upcoming_maintenance): (Vec<ServiceGroup>, Vec<MaintenanceBanner>, Vec<MaintenanceBanner>) = db
        .call(move |conn| {
            let endpoints = crate::db::endpoints::list_all(conn)?;

            let raw_now = chrono::Utc::now();
            let slot_secs = slot_minutes * 60;
            let ts = raw_now.timestamp();
            let remainder = ts % slot_secs;
            let aligned_ts = if remainder == 0 { ts } else { ts + slot_secs - remainder };
            let now = chrono::DateTime::from_timestamp(aligned_ts, 0)
                .unwrap_or(raw_now);

            let start = now - chrono::Duration::hours(hours);
            let start_str = start.format("%Y-%m-%d %H:%M:%S").to_string();

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

                // Absolute uptime: strip _MAINTENANCE suffix, count base state
                // Exclude NO_DATA entries entirely — they are not failures
                let non_nodata_entries: Vec<_> = entries
                    .iter()
                    .filter(|e| {
                        let base = base_state(&e.state);
                        base != "NO_DATA"
                    })
                    .collect();
                let total = non_nodata_entries.len();
                let ok_count = non_nodata_entries
                    .iter()
                    .filter(|e| {
                        let base = base_state(&e.state);
                        base == "OK" || base == "WARNING"
                    })
                    .count();

                // SLA uptime: exclude _MAINTENANCE and NO_DATA entries entirely
                let non_maint_entries: Vec<_> = entries
                    .iter()
                    .filter(|e| {
                        let base = base_state(&e.state);
                        !e.state.ends_with("_MAINTENANCE") && e.state != "MAINTENANCE" && base != "NO_DATA"
                    })
                    .collect();
                let sla_total = non_maint_entries.len();
                let sla_ok_count = non_maint_entries
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
                        sla_entry_counts: (sla_ok_count, sla_total),
                    });
            }

            let mut result = Vec::new();
            for (name, eps) in &by_name {
                // Merge slot states: worst base state wins, maintenance tag preserved
                let mut merged_slots: Vec<String> = vec!["NO_DATA".to_string(); SLOTS];
                for ep in eps {
                    for (i, s) in ep.slot_states.iter().enumerate() {
                        merged_slots[i] = merge_slot_state(&merged_slots[i], s);
                    }
                }

                let timeline = slots_to_timeline(&merged_slots, now, hours, slot_minutes);

                // Absolute uptime: per-endpoint, take the minimum
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

                // SLA uptime
                let sla_uptime_pct = eps
                    .iter()
                    .filter_map(|e| {
                        if e.sla_entry_counts.1 > 0 {
                            Some(e.sla_entry_counts.0 as f64 / e.sla_entry_counts.1 as f64 * 100.0)
                        } else {
                            None
                        }
                    })
                    .fold(f64::MAX, f64::min);
                let sla_uptime_pct = if sla_uptime_pct == f64::MAX { 100.0 } else { sla_uptime_pct };

                // Group current state = worst across checks (using base state for ranking)
                let group_state = eps
                    .iter()
                    .map(|e| e.current_state.as_str())
                    .fold("NO_DATA".to_string(), |a, b| merge_slot_state(&a, b));

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
                    current_state: group_state.clone(),
                    current_state_css: state_to_css(&group_state).to_string(),
                    uptime_pct: format!("{:.2}%", uptime_pct),
                    sla_uptime_pct: format!("{:.2}%", sla_uptime_pct),
                    timeline,
                    checks,
                    expandable,
                });
            }

            // Fetch active & upcoming maintenance windows
            let now_str = raw_now.format("%Y-%m-%d %H:%M:%S").to_string();
            let active_mw = crate::db::maintenance::get_active(conn, &now_str)?;
            let upcoming_mw = crate::db::maintenance::get_upcoming(conn, &now_str, 7)?;

            let resolve_names = |windows: Vec<MaintenanceWindow>| -> Vec<MaintenanceBanner> {
                windows
                    .into_iter()
                    .map(|w| {
                        let names: Vec<String> = w
                            .endpoint_ids
                            .iter()
                            .filter_map(|eid| {
                                endpoints.iter().find(|ep| ep.id == *eid).map(|ep| {
                                    match &ep.subname {
                                        Some(sub) => format!("{} ({})", ep.name, sub),
                                        None => ep.name.clone(),
                                    }
                                })
                            })
                            .collect();
                        MaintenanceBanner {
                            name: w.name,
                            start_time: w.start_time,
                            end_time: w.end_time,
                            endpoint_names: names,
                        }
                    })
                    .collect()
            };

            let active_banners = resolve_names(active_mw);
            let upcoming_banners = resolve_names(upcoming_mw);

            Ok((result, active_banners, upcoming_banners))
        })
        .await?;

    let has_critical = groups.iter().any(|g| {
        let base = base_state(&g.current_state);
        base == "CRITICAL"
    });
    let has_warning = groups.iter().any(|g| {
        let base = base_state(&g.current_state);
        base == "WARNING"
    });
    let has_maintenance = groups.iter().any(|g| g.current_state.contains("MAINTENANCE"));

    let (overall_label, overall_css) = if has_critical {
        ("Some systems are experiencing issues".to_string(), "overall-critical".to_string())
    } else if has_warning {
        ("Some systems have warnings".to_string(), "overall-warning".to_string())
    } else if has_maintenance {
        ("Some systems under planned maintenance".to_string(), "overall-maintenance".to_string())
    } else {
        ("All systems operational".to_string(), "overall-ok".to_string())
    };

    Ok(Html(
        StatusDataTemplate {
            groups,
            overall_label,
            overall_css,
            active_range,
            active_maintenance,
            upcoming_maintenance,
        }
        .render()
        .unwrap_or_default(),
    ))
}

/// Build raw slot states from history entries.
fn build_slot_states(
    entries: &[HistoryEntry],
    now: chrono::DateTime<chrono::Utc>,
    hours: i64,
    slot_minutes: i64,
) -> Vec<String> {
    let start = now - chrono::Duration::hours(hours);
    let mut slot_states: Vec<String> = vec!["NO_DATA".to_string(); SLOTS];

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

        slot_states[slot_idx] = merge_slot_state(&slot_states[slot_idx], &entry.state);
    }

    slot_states
}

/// Convert slot states into timeline slots for rendering.
fn slots_to_timeline(
    slot_states: &[String],
    now: chrono::DateTime<chrono::Utc>,
    hours: i64,
    slot_minutes: i64,
) -> Vec<TimelineSlot> {
    let start = now - chrono::Duration::hours(hours);

    slot_states
        .iter()
        .enumerate()
        .map(|(i, state)| {
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

/// Strip _MAINTENANCE suffix to get the base state.
fn base_state(s: &str) -> &str {
    s.strip_suffix("_MAINTENANCE").unwrap_or(s)
}

/// Merge two states for slot visualization.
/// The worse base state wins. Maintenance tag is preserved if either has it.
fn merge_slot_state(a: &str, b: &str) -> String {
    let base_a = base_state(a);
    let base_b = base_state(b);
    let maint_a = a.ends_with("_MAINTENANCE") || a == "MAINTENANCE";
    let maint_b = b.ends_with("_MAINTENANCE") || b == "MAINTENANCE";
    let either_maint = maint_a || maint_b;

    let rank = |s: &str| match s {
        "CRITICAL" => 4,
        "WARNING" => 3,
        "OK" => 1,
        _ => 0, // NO_DATA, MAINTENANCE
    };

    let worse = if rank(base_b) > rank(base_a) { base_b } else { base_a };

    if either_maint && worse != "NO_DATA" {
        format!("{}_MAINTENANCE", worse)
    } else if either_maint {
        "MAINTENANCE".to_string()
    } else {
        worse.to_string()
    }
}

fn state_to_css(state: &str) -> &str {
    match state {
        "OK" => "ok",
        "OK_MAINTENANCE" => "ok-maintenance",
        "WARNING" => "warning",
        "WARNING_MAINTENANCE" => "warning-maintenance",
        "CRITICAL" => "critical",
        "CRITICAL_MAINTENANCE" => "critical-maintenance",
        "MAINTENANCE" => "maintenance",
        "NO_DATA_MAINTENANCE" => "nodata-maintenance",
        _ => "nodata",
    }
}
