use rusqlite::{params, Connection};

/// Uptime statistics for a single service (grouped by endpoint name).
#[derive(Debug, Clone)]
pub struct ServiceUptime {
    pub name: String,
    pub uptime_pct: f64,
    pub sla_uptime_pct: f64,
    pub total_checks: usize,
    pub critical_checks: usize,
    pub warning_checks: usize,
}

/// Summary of an incident within a report period.
#[derive(Debug, Clone)]
pub struct IncidentSummary {
    pub title: String,
    pub severity: String,
    pub status: String,
    pub endpoint_name: String,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub duration_minutes: i64,
}

/// Summary of a maintenance window within a report period.
#[derive(Debug, Clone)]
pub struct MaintenanceSummary {
    pub name: String,
    pub start_time: String,
    pub end_time: String,
    pub endpoint_names: Vec<String>,
}

/// Compute per-service uptime for a given date range (start/end as "%Y-%m-%d %H:%M:%S").
pub fn compute_service_uptimes(
    conn: &Connection,
    start: &str,
    end: &str,
) -> rusqlite::Result<Vec<ServiceUptime>> {
    let endpoints = crate::db::endpoints::list_all(conn)?;

    // Group endpoints by name
    let mut by_name: std::collections::BTreeMap<String, Vec<&crate::db::endpoints::Endpoint>> =
        std::collections::BTreeMap::new();
    for ep in &endpoints {
        if ep.enabled {
            by_name.entry(ep.name.clone()).or_default().push(ep);
        }
    }

    let mut results = Vec::new();

    for (name, eps) in &by_name {
        let mut group_uptime = f64::MAX;
        let mut group_sla = f64::MAX;
        let mut group_total = 0usize;
        let mut group_critical = 0usize;
        let mut group_warning = 0usize;

        for ep in eps {
            let mut stmt = conn.prepare(
                "SELECT state FROM state_history
                 WHERE endpoint_id = ?1 AND timestamp >= ?2 AND timestamp < ?3
                 ORDER BY timestamp ASC",
            )?;
            let states: Vec<String> = stmt
                .query_map(params![ep.id, start, end], |row| row.get("state"))?
                .collect::<Result<Vec<_>, _>>()?;

            let total = states.len();
            if total == 0 {
                continue;
            }

            let mut ok_count = 0usize;
            let mut critical_count = 0usize;
            let mut warning_count = 0usize;
            let mut non_nodata = 0usize;
            let mut sla_ok = 0usize;
            let mut sla_total = 0usize;

            for s in &states {
                let base = s.strip_suffix("_MAINTENANCE").unwrap_or(s);
                let is_maint = s.ends_with("_MAINTENANCE") || s == "MAINTENANCE";

                if base == "NO_DATA" {
                    continue;
                }
                non_nodata += 1;

                match base {
                    "OK" | "WARNING" => ok_count += 1,
                    "CRITICAL" => critical_count += 1,
                    _ => {}
                }
                if base == "WARNING" {
                    warning_count += 1;
                }

                if !is_maint {
                    sla_total += 1;
                    if base == "OK" || base == "WARNING" {
                        sla_ok += 1;
                    }
                }
            }

            group_total += total;
            group_critical += critical_count;
            group_warning += warning_count;

            let ep_uptime = if non_nodata > 0 {
                ok_count as f64 / non_nodata as f64 * 100.0
            } else {
                100.0
            };
            let ep_sla = if sla_total > 0 {
                sla_ok as f64 / sla_total as f64 * 100.0
            } else {
                100.0
            };

            group_uptime = group_uptime.min(ep_uptime);
            group_sla = group_sla.min(ep_sla);
        }

        if group_uptime == f64::MAX {
            group_uptime = 100.0;
        }
        if group_sla == f64::MAX {
            group_sla = 100.0;
        }

        results.push(ServiceUptime {
            name: name.clone(),
            uptime_pct: group_uptime,
            sla_uptime_pct: group_sla,
            total_checks: group_total,
            critical_checks: group_critical,
            warning_checks: group_warning,
        });
    }

    Ok(results)
}

/// List incidents that overlap with the given date range.
pub fn list_incidents_in_range(
    conn: &Connection,
    start: &str,
    end: &str,
) -> rusqlite::Result<Vec<IncidentSummary>> {
    let endpoints = crate::db::endpoints::list_all(conn)?;
    let ep_name = |eid: i64| -> String {
        endpoints
            .iter()
            .find(|e| e.id == eid)
            .map(|e| match &e.subname {
                Some(sub) => format!("{} ({})", e.name, sub),
                None => e.name.clone(),
            })
            .unwrap_or_else(|| format!("Endpoint #{eid}"))
    };

    let mut stmt = conn.prepare(
        "SELECT endpoint_id, title, severity, status, created_at, resolved_at
         FROM incidents
         WHERE created_at < ?2
           AND (resolved_at IS NULL OR resolved_at >= ?1)
         ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map(params![start, end], |row| {
        let endpoint_id: i64 = row.get("endpoint_id")?;
        let created_at: String = row.get("created_at")?;
        let resolved_at: Option<String> = row.get("resolved_at")?;
        let dur = compute_duration_minutes(&created_at, resolved_at.as_deref());
        Ok(IncidentSummary {
            title: row.get("title")?,
            severity: row.get("severity")?,
            status: row.get("status")?,
            endpoint_name: ep_name(endpoint_id),
            created_at,
            resolved_at,
            duration_minutes: dur,
        })
    })?;
    rows.collect()
}

/// List maintenance windows that overlap with the given date range.
pub fn list_maintenance_in_range(
    conn: &Connection,
    start: &str,
    end: &str,
) -> rusqlite::Result<Vec<MaintenanceSummary>> {
    let endpoints = crate::db::endpoints::list_all(conn)?;

    let mut stmt = conn.prepare(
        "SELECT mw.id, mw.name, mw.start_time, mw.end_time
         FROM maintenance_windows mw
         WHERE mw.start_time < ?2 AND mw.end_time > ?1
         ORDER BY mw.start_time ASC",
    )?;
    let windows: Vec<(i64, String, String, String)> = stmt
        .query_map(params![start, end], |row| {
            Ok((
                row.get("id")?,
                row.get("name")?,
                row.get("start_time")?,
                row.get("end_time")?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut results = Vec::new();
    for (id, name, start_time, end_time) in windows {
        let mut ep_stmt = conn.prepare(
            "SELECT endpoint_id FROM maintenance_window_endpoints WHERE window_id = ?1",
        )?;
        let ep_ids: Vec<i64> = ep_stmt
            .query_map(params![id], |row| row.get("endpoint_id"))?
            .collect::<Result<Vec<_>, _>>()?;

        let ep_names: Vec<String> = ep_ids
            .iter()
            .filter_map(|eid| {
                endpoints.iter().find(|e| e.id == *eid).map(|e| match &e.subname {
                    Some(sub) => format!("{} ({})", e.name, sub),
                    None => e.name.clone(),
                })
            })
            .collect();

        results.push(MaintenanceSummary {
            name,
            start_time,
            end_time,
            endpoint_names: ep_names,
        });
    }

    Ok(results)
}

fn compute_duration_minutes(start: &str, end: Option<&str>) -> i64 {
    let parse = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok();
    match (parse(start), end.and_then(parse)) {
        (Some(s), Some(e)) => (e - s).num_minutes(),
        _ => 0,
    }
}
