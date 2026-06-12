// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2026 Collabora Ltd.
// Author: Denys Fedoryshchenko <denys.f@collabora.com>

use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::collections::HashMap;

use super::NotificationEvent;

/// Build the SMTP transport from the config map.
fn build_transport(
    config: &HashMap<String, String>,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    let host = config
        .get("smtp_host")
        .filter(|h| !h.is_empty())
        .ok_or("SMTP server is not configured")?;
    let port: u16 = config
        .get("smtp_port")
        .unwrap_or(&"587".to_string())
        .parse()
        .map_err(|_| "Invalid SMTP port")?;
    let username = config
        .get("smtp_username")
        .filter(|u| !u.is_empty())
        .ok_or("SMTP username is not configured")?;
    let password = config
        .get("smtp_password")
        .filter(|p| !p.is_empty())
        .ok_or("SMTP password is not configured")?;

    let creds = Credentials::new(username.clone(), password.clone());

    let use_ssl = config.get("smtp_ssl").is_some_and(|v| v == "true");

    let transport = if use_ssl {
        // Implicit TLS (port 465 typically)
        AsyncSmtpTransport::<Tokio1Executor>::relay(host)
            .map_err(|e| e.to_string())?
            .port(port)
            .credentials(creds)
            .build()
    } else {
        // STARTTLS (port 587 typically) — this is also the default / fallback
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| e.to_string())?
            .port(port)
            .credentials(creds)
            .build()
    };

    Ok(transport)
}

/// Parse the "From" mailbox from config (email + optional display name).
fn from_mailbox(config: &HashMap<String, String>) -> Result<Mailbox, String> {
    let from_email = config
        .get("email_from")
        .filter(|e| !e.is_empty())
        .ok_or("From email address is not configured")?;
    let from_name = config.get("email_from_name").cloned().unwrap_or_default();

    if from_name.is_empty() {
        from_email
            .parse()
            .map_err(|e| format!("Invalid From address: {e}"))
    } else {
        format!("{from_name} <{from_email}>")
            .parse()
            .map_err(|e| format!("Invalid From address: {e}"))
    }
}

/// Parse a comma-separated recipient list (from the given config key) into Mailboxes.
fn recipient_mailboxes_from(
    config: &HashMap<String, String>,
    key: &str,
) -> Result<Vec<Mailbox>, String> {
    let to_str = config
        .get(key)
        .filter(|t| !t.is_empty())
        .ok_or("No recipient email addresses configured")?;

    let mut mailboxes = Vec::new();
    for addr in to_str.split(',') {
        let addr = addr.trim();
        if addr.is_empty() {
            continue;
        }
        let mbox: Mailbox = addr
            .parse()
            .map_err(|e| format!("Invalid recipient '{addr}': {e}"))?;
        mailboxes.push(mbox);
    }
    if mailboxes.is_empty() {
        return Err("No valid recipient email addresses".to_string());
    }
    Ok(mailboxes)
}

/// Parse the default ("email_to") recipient list.
fn recipient_mailboxes(config: &HashMap<String, String>) -> Result<Vec<Mailbox>, String> {
    recipient_mailboxes_from(config, "email_to")
}

/// Pick the recipient-list config key for a given state transition.
/// WARNING ↔ OK transitions go to the warnings list; everything else
/// (anything involving CRITICAL, or unknown states) goes to the default list.
fn recipient_key_for_transition(old_state: &str, new_state: &str) -> &'static str {
    match super::transition_kind(old_state, new_state) {
        "warning" => "email_to_warnings",
        _ => "email_to",
    }
}

/// Send a status-change notification email.
pub async fn send_notification(
    config: &HashMap<String, String>,
    event: &NotificationEvent,
) -> Result<(), String> {
    let key = recipient_key_for_transition(&event.old_state, &event.new_state);
    // Empty recipient list is a valid "suppress" signal — especially for the
    // warnings list, which users may leave blank to silence warning emails.
    if config.get(key).map(|s| s.trim().is_empty()).unwrap_or(true) {
        return Ok(());
    }

    let transport = build_transport(config)?;
    let from = from_mailbox(config)?;
    let recipients = recipient_mailboxes_from(config, key)?;

    let display_name = match &event.subname {
        Some(sub) => format!("{} ({})", event.endpoint_name, sub),
        None => event.endpoint_name.clone(),
    };

    let severity = match event.new_state.as_str() {
        "CRITICAL" => "CRITICAL",
        "WARNING" => "WARNING",
        "OK" => "OK",
        _ => "service",
    };
    let (state_emoji, state_color) = match event.new_state.as_str() {
        "OK" => ("\u{2705}", "#16a34a"),
        "WARNING" => ("\u{26a0}\u{fe0f}", "#ca8a04"),
        "CRITICAL" => ("\u{274c}", "#dc2626"),
        _ => ("\u{2753}", "#6b7280"),
    };

    let subject = format!(
        "[{severity}] {display_name}: {} \u{2192} {}",
        event.old_state, event.new_state
    );

    let value_line = event
        .value
        .as_ref()
        .map(|v| format!("<tr><td style=\"padding:6px 12px;color:#6b7280;\">Value</td><td style=\"padding:6px 12px;\">{v}</td></tr>"))
        .unwrap_or_default();
    let message_line = event
        .message
        .as_ref()
        .filter(|m| !m.is_empty())
        .map(|m| format!("<tr><td style=\"padding:6px 12px;color:#6b7280;\">Details</td><td style=\"padding:6px 12px;\">{m}</td></tr>"))
        .unwrap_or_default();

    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");

    let html_body = format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="UTF-8"></head>
<body style="margin:0;padding:0;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;background:#f3f4f6;">
<div style="max-width:600px;margin:24px auto;background:#fff;border-radius:8px;overflow:hidden;border:1px solid #e5e7eb;">
  <div style="background:#001B71;padding:16px 24px;">
    <h1 style="margin:0;color:#fff;font-size:18px;">KernelCI Status Alert</h1>
  </div>
  <div style="padding:24px;">
    <div style="display:flex;align-items:center;gap:8px;margin-bottom:16px;">
      <span style="font-size:28px;">{state_emoji}</span>
      <div>
        <div style="font-size:18px;font-weight:600;color:#111;">{display_name}</div>
        <div style="font-size:14px;color:#6b7280;">Status changed at {timestamp}</div>
      </div>
    </div>
    <table style="width:100%;border-collapse:collapse;margin:16px 0;border:1px solid #e5e7eb;border-radius:6px;">
      <tr>
        <td style="padding:6px 12px;color:#6b7280;">Previous State</td>
        <td style="padding:6px 12px;font-weight:600;">{old_state}</td>
      </tr>
      <tr style="background:#f9fafb;">
        <td style="padding:6px 12px;color:#6b7280;">Current State</td>
        <td style="padding:6px 12px;font-weight:700;color:{state_color};">{new_state}</td>
      </tr>
      {value_line}
      {message_line}
    </table>
    <div style="font-size:12px;color:#9ca3af;margin-top:24px;border-top:1px solid #e5e7eb;padding-top:12px;">
      Sent by KernelCI Status Monitoring
    </div>
  </div>
</div>
</body>
</html>"#,
        old_state = event.old_state,
        new_state = event.new_state,
    );

    for recipient in &recipients {
        let email = Message::builder()
            .from(from.clone())
            .to(recipient.clone())
            .subject(&subject)
            .header(ContentType::TEXT_HTML)
            .body(html_body.clone())
            .map_err(|e| e.to_string())?;

        transport.send(email).await.map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Data needed to compose an incident email.
pub struct IncidentEmailData {
    pub title: String,
    pub endpoint_name: String,
    pub severity: String,
    pub status: String,
}

pub struct ActionLink {
    pub label: String,
    pub url: String,
}

/// Send an incident notification email to a single recipient.
pub async fn send_incident_email(
    config: &HashMap<String, String>,
    to_email: &str,
    to_name: &str,
    subject: &str,
    incident: &IncidentEmailData,
    action_links: &[ActionLink],
    extra_message: &str,
) -> Result<(), String> {
    let transport = build_transport(config)?;
    let from = from_mailbox(config)?;

    let to_mbox: Mailbox = to_email
        .parse()
        .map_err(|e| format!("Invalid recipient: {e}"))?;

    let (severity_emoji, severity_color) = match incident.severity.as_str() {
        "critical" => ("\u{274c}", "#dc2626"),
        _ => ("\u{26a0}\u{fe0f}", "#ca8a04"),
    };

    let links_html = if action_links.is_empty() {
        String::new()
    } else {
        let btns: Vec<String> = action_links
            .iter()
            .map(|l| {
                format!(
                    r#"<a href="{url}" style="display:inline-block;background:#001B71;color:#fff;padding:10px 24px;border-radius:5px;text-decoration:none;font-weight:600;margin:4px 8px 4px 0;">{label}</a>"#,
                    url = l.url,
                    label = l.label,
                )
            })
            .collect();
        format!(r#"<div style="margin:20px 0;">{}</div>"#, btns.join("\n"))
    };

    let extra_html = if extra_message.is_empty() {
        String::new()
    } else {
        format!(r#"<p style="color:#374151;margin:12px 0;">{extra_message}</p>"#)
    };

    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");

    let html_body = format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="UTF-8"></head>
<body style="margin:0;padding:0;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;background:#f3f4f6;">
<div style="max-width:600px;margin:24px auto;background:#fff;border-radius:8px;overflow:hidden;border:1px solid #e5e7eb;">
  <div style="background:#001B71;padding:16px 24px;">
    <h1 style="margin:0;color:#fff;font-size:18px;">KernelCI Incident Alert</h1>
  </div>
  <div style="padding:24px;">
    <p style="color:#6b7280;font-size:14px;margin-bottom:4px;">Hi {to_name},</p>
    {extra_html}
    <div style="display:flex;align-items:center;gap:8px;margin:16px 0;">
      <span style="font-size:28px;">{severity_emoji}</span>
      <div>
        <div style="font-size:18px;font-weight:600;color:#111;">{title}</div>
        <div style="font-size:14px;color:#6b7280;">{timestamp}</div>
      </div>
    </div>
    <table style="width:100%;border-collapse:collapse;margin:16px 0;border:1px solid #e5e7eb;border-radius:6px;">
      <tr>
        <td style="padding:6px 12px;color:#6b7280;">Endpoint</td>
        <td style="padding:6px 12px;font-weight:600;">{endpoint_name}</td>
      </tr>
      <tr style="background:#f9fafb;">
        <td style="padding:6px 12px;color:#6b7280;">Severity</td>
        <td style="padding:6px 12px;font-weight:700;color:{severity_color};">{severity}</td>
      </tr>
      <tr>
        <td style="padding:6px 12px;color:#6b7280;">Status</td>
        <td style="padding:6px 12px;">{status}</td>
      </tr>
    </table>
    {links_html}
    <div style="font-size:12px;color:#9ca3af;margin-top:24px;border-top:1px solid #e5e7eb;padding-top:12px;">
      Sent by KernelCI Status Monitoring
    </div>
  </div>
</div>
</body>
</html>"#,
        title = incident.title,
        endpoint_name = incident.endpoint_name,
        severity = incident.severity,
        status = incident.status,
    );

    let email = Message::builder()
        .from(from)
        .to(to_mbox)
        .subject(subject)
        .header(ContentType::TEXT_HTML)
        .body(html_body)
        .map_err(|e| e.to_string())?;

    transport.send(email).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Data for a maintenance reminder email.
pub struct MaintenanceReminderData {
    pub window_name: String,
    pub start_time: String,
    pub end_time: String,
    pub endpoint_names: Vec<String>,
    pub is_deploy: bool,
    pub changelog: Option<String>,
}

/// Send a maintenance reminder email to notification recipients.
pub async fn send_maintenance_reminder(
    config: &HashMap<String, String>,
    data: &MaintenanceReminderData,
) -> Result<(), String> {
    let transport = build_transport(config)?;
    let from = from_mailbox(config)?;
    let recipients = recipient_mailboxes(config)?;

    let deploy_label = if data.is_deploy { " (Deploy)" } else { "" };

    let subject = format!(
        "[maintenance] {}{} — starting in less than 1 hour",
        data.window_name, deploy_label
    );

    let endpoints_html = if data.endpoint_names.is_empty() {
        "None specified".to_string()
    } else {
        data.endpoint_names.join(", ")
    };

    let changelog_html = match &data.changelog {
        Some(cl) if !cl.trim().is_empty() => {
            let escaped = cl
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\n', "<br>");
            format!(
                r#"<tr><td style="padding:6px 12px;color:#6b7280;vertical-align:top;">Changelog</td><td style="padding:6px 12px;font-family:monospace;font-size:13px;">{escaped}</td></tr>"#
            )
        }
        _ => String::new(),
    };

    let html_body = format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="UTF-8"></head>
<body style="margin:0;padding:0;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;background:#f3f4f6;">
<div style="max-width:600px;margin:24px auto;background:#fff;border-radius:8px;overflow:hidden;border:1px solid #e5e7eb;">
  <div style="background:#001B71;padding:16px 24px;">
    <h1 style="margin:0;color:#fff;font-size:18px;">KernelCI Maintenance Reminder</h1>
  </div>
  <div style="padding:24px;">
    <div style="display:flex;align-items:center;gap:8px;margin-bottom:16px;">
      <span style="font-size:28px;">&#128336;</span>
      <div>
        <div style="font-size:18px;font-weight:600;color:#111;">{name}</div>
        <div style="font-size:14px;color:#6b7280;">Starting in less than 1 hour</div>
      </div>
    </div>
    <table style="width:100%;border-collapse:collapse;margin:16px 0;border:1px solid #e5e7eb;border-radius:6px;">
      <tr>
        <td style="padding:6px 12px;color:#6b7280;">Start</td>
        <td style="padding:6px 12px;font-weight:600;">{start_time} UTC</td>
      </tr>
      <tr style="background:#f9fafb;">
        <td style="padding:6px 12px;color:#6b7280;">End</td>
        <td style="padding:6px 12px;font-weight:600;">{end_time} UTC</td>
      </tr>
      <tr>
        <td style="padding:6px 12px;color:#6b7280;">Affected</td>
        <td style="padding:6px 12px;">{endpoints}</td>
      </tr>
      {changelog_section}
    </table>
    <div style="font-size:12px;color:#9ca3af;margin-top:24px;border-top:1px solid #e5e7eb;padding-top:12px;">
      Sent by KernelCI Status Monitoring
    </div>
  </div>
</div>
</body>
</html>"#,
        name = data.window_name,
        start_time = data.start_time,
        end_time = data.end_time,
        endpoints = endpoints_html,
        changelog_section = changelog_html,
    );

    for recipient in &recipients {
        let email = Message::builder()
            .from(from.clone())
            .to(recipient.clone())
            .subject(&subject)
            .header(ContentType::TEXT_HTML)
            .body(html_body.clone())
            .map_err(|e| e.to_string())?;

        transport.send(email).await.map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Send a plain-text test email to verify SMTP configuration.
pub async fn send_test(config: &HashMap<String, String>) -> Result<(), String> {
    let transport = build_transport(config)?;
    let from = from_mailbox(config)?;
    let recipients = recipient_mailboxes(config)?;

    let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    let body = format!(
        "This is a test email from KernelCI Status Monitoring.\n\n\
         If you received this message, your SMTP settings are configured correctly.\n\n\
         Sent at: {timestamp}"
    );

    for recipient in &recipients {
        let email = Message::builder()
            .from(from.clone())
            .to(recipient.clone())
            .subject("KernelCI Status — Test Email")
            .header(ContentType::TEXT_PLAIN)
            .body(body.clone())
            .map_err(|e| e.to_string())?;

        transport.send(email).await.map_err(|e| e.to_string())?;
    }

    Ok(())
}
