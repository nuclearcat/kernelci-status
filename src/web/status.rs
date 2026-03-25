use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse};

use crate::db::history::HistoryEntry;
use crate::error::AppError;
use crate::state::AppState;

/// Number of 15-minute slots in 24 hours.
const SLOTS: usize = 96;

/// One slot in the 24h timeline bar.
pub struct TimelineSlot {
    pub css_class: String,
    pub title: String,
}

/// A service row on the public status page.
pub struct ServiceStatus {
    pub name: String,
    pub subname: String,
    pub current_state: String,
    pub current_state_css: String,
    pub uptime_pct: String,
    pub timeline: Vec<TimelineSlot>,
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
    services: Vec<ServiceStatus>,
    overall_label: String,
    overall_css: String,
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

/// Serve the status data fragment (called by HTMX on load + every 60s).
pub async fn status_data(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let db = state.db.clone();
    let services: Vec<ServiceStatus> = db
        .call(|conn| {
            let endpoints = crate::db::endpoints::list_all(conn)?;
            let now = chrono::Utc::now();
            let mut result = Vec::new();

            for ep in &endpoints {
                if !ep.enabled {
                    continue;
                }
                let entries = crate::db::history::get_last_hours(conn, ep.id, 24)?;
                let latest = crate::db::history::get_latest_for_endpoint(conn, ep.id)?;

                let current_state = latest
                    .as_ref()
                    .map(|h| h.state.as_str())
                    .unwrap_or("NO_DATA");

                let (timeline, uptime_pct) = build_timeline(&entries, now);

                result.push(ServiceStatus {
                    name: ep.name.clone(),
                    subname: ep.subname.clone().unwrap_or_default(),
                    current_state: current_state.to_string(),
                    current_state_css: state_to_css(current_state).to_string(),
                    uptime_pct: format!("{:.1}%", uptime_pct),
                    timeline,
                });
            }
            Ok(result)
        })
        .await?;

    let has_critical = services.iter().any(|s| s.current_state == "CRITICAL");
    let has_warning = services.iter().any(|s| s.current_state == "WARNING");

    let (overall_label, overall_css) = if has_critical {
        ("Some systems are experiencing issues".to_string(), "overall-critical".to_string())
    } else if has_warning {
        ("Some systems have warnings".to_string(), "overall-warning".to_string())
    } else {
        ("All systems operational".to_string(), "overall-ok".to_string())
    };

    Ok(Html(
        StatusDataTemplate {
            services,
            overall_label,
            overall_css,
        }
        .render()
        .unwrap_or_default(),
    ))
}

/// Build a 96-slot timeline from history entries and compute uptime %.
fn build_timeline(
    entries: &[HistoryEntry],
    now: chrono::DateTime<chrono::Utc>,
) -> (Vec<TimelineSlot>, f64) {
    let start = now - chrono::Duration::hours(24);

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
        let slot_idx = (elapsed.num_minutes() / 15) as usize;
        if slot_idx >= SLOTS {
            continue;
        }

        let current_worst = slot_states[slot_idx];
        slot_states[slot_idx] = worst_state(current_worst, &entry.state);
    }

    let slots_with_data = slot_states.iter().filter(|&&s| s != "NO_DATA").count();
    let slots_ok = slot_states.iter().filter(|&&s| s == "OK").count();
    let uptime_pct = if slots_with_data > 0 {
        (slots_ok as f64 / slots_with_data as f64) * 100.0
    } else {
        100.0
    };

    let timeline: Vec<TimelineSlot> = slot_states
        .iter()
        .enumerate()
        .map(|(i, &state)| {
            let slot_time = start + chrono::Duration::minutes(i as i64 * 15);
            let time_str = slot_time.format("%H:%M").to_string();
            TimelineSlot {
                css_class: format!("slot-{}", state_to_css(state)),
                title: format!("{} — {}", time_str, state),
            }
        })
        .collect();

    (timeline, uptime_pct)
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
