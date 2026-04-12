use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::Form;
use chrono::Datelike;
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::db::reports::{IncidentSummary, MaintenanceSummary, ServiceUptime};
use crate::error::AppError;
use crate::state::AppState;

/// View model for a service row in the report.
pub struct ReportService {
    pub name: String,
    pub uptime_pct: String,
    pub sla_uptime_pct: String,
    pub uptime_css: String,
    pub total_checks: usize,
    pub critical_checks: usize,
    pub warning_checks: usize,
}

/// View model for an incident in the report.
pub struct ReportIncident {
    pub title: String,
    pub severity: String,
    pub severity_css: String,
    pub status: String,
    pub endpoint_name: String,
    pub created_at: String,
    pub resolved_at: String,
    pub duration: String,
}

/// View model for a maintenance window in the report.
pub struct ReportMaintenance {
    pub name: String,
    pub start_time: String,
    pub end_time: String,
    pub endpoints: String,
}

#[derive(Template)]
#[template(path = "reports.html")]
struct ReportsTemplate {
    username: String,
    report_weekly_enabled: bool,
    report_weekly_day: String,
    report_monthly_enabled: bool,
    report_monthly_day: String,
    error: String,
    success: String,
}

#[derive(Template)]
#[template(path = "report_preview.html")]
struct ReportPreviewTemplate {
    username: String,
    report_type: String,
    period_start: String,
    period_end: String,
    generated_at: String,
    overall_uptime: String,
    overall_sla: String,
    overall_css: String,
    services: Vec<ReportService>,
    incidents: Vec<ReportIncident>,
    total_incidents: usize,
    critical_incidents: usize,
    resolved_incidents: usize,
    maintenance_windows: Vec<ReportMaintenance>,
    total_maintenance: usize,
}

pub async fn reports_page(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<impl IntoResponse, AppError> {
    let config = state.config_cache.read().await;
    let weekly_enabled = config.get("report_weekly_enabled").is_some_and(|v| v == "true");
    let weekly_day = config
        .get("report_weekly_day")
        .cloned()
        .unwrap_or_else(|| "1".to_string());
    let monthly_enabled = config.get("report_monthly_enabled").is_some_and(|v| v == "true");
    let monthly_day = config
        .get("report_monthly_day")
        .cloned()
        .unwrap_or_else(|| "1".to_string());

    Ok(Html(
        ReportsTemplate {
            username: user.username,
            report_weekly_enabled: weekly_enabled,
            report_weekly_day: weekly_day,
            report_monthly_enabled: monthly_enabled,
            report_monthly_day: monthly_day,
            error: String::new(),
            success: String::new(),
        }
        .render()
        .unwrap_or_default(),
    ))
}

#[derive(Deserialize)]
pub struct ReportScheduleForm {
    pub report_weekly_enabled: Option<String>,
    pub report_weekly_day: Option<String>,
    pub report_monthly_enabled: Option<String>,
    pub report_monthly_day: Option<String>,
}

pub async fn save_report_schedule(
    State(state): State<AppState>,
    user: AuthUser,
    Form(form): Form<ReportScheduleForm>,
) -> Result<impl IntoResponse, AppError> {
    let weekly_day = form
        .report_weekly_day
        .clone()
        .unwrap_or_else(|| "1".to_string());
    let monthly_day = form
        .report_monthly_day
        .clone()
        .unwrap_or_else(|| "1".to_string());

    // Validate
    let wd: u8 = weekly_day.parse().unwrap_or(0);
    if wd > 6 {
        return Ok(Html(
            ReportsTemplate {
                username: user.username,
                report_weekly_enabled: form.report_weekly_enabled.as_deref() == Some("on"),
                report_weekly_day: weekly_day,
                report_monthly_enabled: form.report_monthly_enabled.as_deref() == Some("on"),
                report_monthly_day: monthly_day,
                error: "Weekly day must be 0 (Monday) to 6 (Sunday).".to_string(),
                success: String::new(),
            }
            .render()
            .unwrap_or_default(),
        ));
    }

    let md: u8 = monthly_day.parse().unwrap_or(0);
    if md < 1 || md > 28 {
        return Ok(Html(
            ReportsTemplate {
                username: user.username,
                report_weekly_enabled: form.report_weekly_enabled.as_deref() == Some("on"),
                report_weekly_day: weekly_day,
                report_monthly_enabled: form.report_monthly_enabled.as_deref() == Some("on"),
                report_monthly_day: monthly_day,
                error: "Monthly day must be 1 to 28.".to_string(),
                success: String::new(),
            }
            .render()
            .unwrap_or_default(),
        ));
    }

    let weekly_enabled = form.report_weekly_enabled.as_deref() == Some("on");
    let monthly_enabled = form.report_monthly_enabled.as_deref() == Some("on");

    let db = state.db.clone();
    let we = if weekly_enabled { "true" } else { "false" }.to_string();
    let me = if monthly_enabled { "true" } else { "false" }.to_string();
    let wd_s = weekly_day.clone();
    let md_s = monthly_day.clone();
    db.call(move |conn| {
        crate::db::config::set(conn, "report_weekly_enabled", &we)?;
        crate::db::config::set(conn, "report_weekly_day", &wd_s)?;
        crate::db::config::set(conn, "report_monthly_enabled", &me)?;
        crate::db::config::set(conn, "report_monthly_day", &md_s)?;
        Ok(())
    })
    .await?;

    // Update config cache
    {
        let mut cache = state.config_cache.write().await;
        cache.insert(
            "report_weekly_enabled".to_string(),
            if weekly_enabled { "true" } else { "false" }.to_string(),
        );
        cache.insert("report_weekly_day".to_string(), weekly_day.clone());
        cache.insert(
            "report_monthly_enabled".to_string(),
            if monthly_enabled { "true" } else { "false" }.to_string(),
        );
        cache.insert("report_monthly_day".to_string(), monthly_day.clone());
    }

    Ok(Html(
        ReportsTemplate {
            username: user.username,
            report_weekly_enabled: weekly_enabled,
            report_weekly_day: weekly_day,
            report_monthly_enabled: monthly_enabled,
            report_monthly_day: monthly_day,
            error: String::new(),
            success: "Report schedule saved.".to_string(),
        }
        .render()
        .unwrap_or_default(),
    ))
}

#[derive(Deserialize)]
pub struct PreviewQuery {
    pub report_type: Option<String>,
}

pub async fn report_preview(
    State(state): State<AppState>,
    user: AuthUser,
    Form(query): Form<PreviewQuery>,
) -> Result<impl IntoResponse, AppError> {
    let report_type = query.report_type.unwrap_or_else(|| "weekly".to_string());
    let now = chrono::Utc::now();
    let generated_at = now.format("%Y-%m-%d %H:%M UTC").to_string();

    let (period_start, period_end, label) = match report_type.as_str() {
        "monthly" => {
            // Previous calendar month
            let first_of_this_month = now
                .date_naive()
                .with_day(1)
                .unwrap_or(now.date_naive());
            let last_month_end = first_of_this_month;
            let last_month_start = (last_month_end - chrono::Duration::days(1))
                .with_day(1)
                .unwrap_or(last_month_end - chrono::Duration::days(28));
            (
                last_month_start.format("%Y-%m-%d 00:00:00").to_string(),
                last_month_end.format("%Y-%m-%d 00:00:00").to_string(),
                "Monthly".to_string(),
            )
        }
        _ => {
            // Previous 7 days
            let end = now.date_naive();
            let start = end - chrono::Duration::days(7);
            (
                start.format("%Y-%m-%d 00:00:00").to_string(),
                end.format("%Y-%m-%d 00:00:00").to_string(),
                "Weekly".to_string(),
            )
        }
    };

    let ps: String = period_start.clone();
    let pe: String = period_end.clone();
    let db = state.db.clone();
    let (uptimes, incidents, maint_windows): (
        Vec<ServiceUptime>,
        Vec<IncidentSummary>,
        Vec<MaintenanceSummary>,
    ) = db
        .call(move |conn| {
            let u = crate::db::reports::compute_service_uptimes(conn, &ps, &pe)?;
            let i = crate::db::reports::list_incidents_in_range(conn, &ps, &pe)?;
            let m = crate::db::reports::list_maintenance_in_range(conn, &ps, &pe)?;
            Ok((u, i, m))
        })
        .await?;

    // Overall uptime = minimum across all services
    let overall_uptime = uptimes
        .iter()
        .map(|s| s.uptime_pct)
        .fold(f64::MAX, f64::min);
    let overall_uptime = if overall_uptime == f64::MAX {
        100.0
    } else {
        overall_uptime
    };

    let overall_sla = uptimes
        .iter()
        .map(|s| s.sla_uptime_pct)
        .fold(f64::MAX, f64::min);
    let overall_sla = if overall_sla == f64::MAX {
        100.0
    } else {
        overall_sla
    };

    let overall_css = if overall_uptime >= 99.9 {
        "ok"
    } else if overall_uptime >= 99.0 {
        "warning"
    } else {
        "critical"
    };

    let services: Vec<ReportService> = uptimes
        .into_iter()
        .map(|s| {
            let css = if s.uptime_pct >= 99.9 {
                "ok"
            } else if s.uptime_pct >= 99.0 {
                "warning"
            } else {
                "critical"
            };
            ReportService {
                name: s.name,
                uptime_pct: format!("{:.3}%", s.uptime_pct),
                sla_uptime_pct: format!("{:.3}%", s.sla_uptime_pct),
                uptime_css: css.to_string(),
                total_checks: s.total_checks,
                critical_checks: s.critical_checks,
                warning_checks: s.warning_checks,
            }
        })
        .collect();

    let total_incidents = incidents.len();
    let critical_incidents = incidents.iter().filter(|i| i.severity == "critical").count();
    let resolved_incidents = incidents.iter().filter(|i| i.status == "resolved").count();

    let report_incidents: Vec<ReportIncident> = incidents
        .into_iter()
        .map(|i| {
            let sev_css = match i.severity.as_str() {
                "critical" => "critical",
                _ => "warning",
            };
            ReportIncident {
                title: i.title,
                severity: i.severity,
                severity_css: sev_css.to_string(),
                status: i.status,
                endpoint_name: i.endpoint_name,
                created_at: i.created_at,
                resolved_at: i.resolved_at.unwrap_or_else(|| "Ongoing".to_string()),
                duration: format_duration(i.duration_minutes),
            }
        })
        .collect();

    let total_maintenance = maint_windows.len();
    let report_maintenance: Vec<ReportMaintenance> = maint_windows
        .into_iter()
        .map(|m| ReportMaintenance {
            name: m.name,
            start_time: m.start_time,
            end_time: m.end_time,
            endpoints: m.endpoint_names.join(", "),
        })
        .collect();

    let period_start_display = &period_start[..10];
    let period_end_display = &period_end[..10];

    Ok(Html(
        ReportPreviewTemplate {
            username: user.username,
            report_type: label,
            period_start: period_start_display.to_string(),
            period_end: period_end_display.to_string(),
            generated_at,
            overall_uptime: format!("{:.3}%", overall_uptime),
            overall_sla: format!("{:.3}%", overall_sla),
            overall_css: overall_css.to_string(),
            services,
            incidents: report_incidents,
            total_incidents,
            critical_incidents,
            resolved_incidents,
            maintenance_windows: report_maintenance,
            total_maintenance,
        }
        .render()
        .unwrap_or_default(),
    ))
}

fn format_duration(minutes: i64) -> String {
    if minutes <= 0 {
        return "Ongoing".to_string();
    }
    if minutes < 60 {
        format!("{minutes} min")
    } else if minutes < 1440 {
        let h = minutes / 60;
        let m = minutes % 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h {m}m")
        }
    } else {
        let d = minutes / 1440;
        let h = (minutes % 1440) / 60;
        if h == 0 {
            format!("{d}d")
        } else {
            format!("{d}d {h}h")
        }
    }
}
