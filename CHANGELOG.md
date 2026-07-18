<!--
SPDX-License-Identifier: LGPL-2.1-only
SPDX-FileCopyrightText: 2026 Collabora Ltd.
Author: Denys Fedoryshchenko <denys.f@collabora.com>
-->

# Changelog

## Unreleased

### Added
- Telegram bot notification backend for status-change alerts (Bot Token + Chat ID on the Notifications page); bot token is write-only in the UI and masked in `GET /api/config`
- `passwd <username>` CLI command to reset an existing user's password and invalidate their sessions

## 0.1.3

### Added
- KernelCI logo on public status page header and admin nav bar
- Service grouping: endpoints with the same name are merged into a single row with aggregated worst-state timeline
- Expandable group rows: click to reveal individual check statuses when a group has multiple checks
- Time range selector (24h / 7d / 30d) on public status page, replaces per-row "24h ago" labels
- Clone button on Endpoints page (creates a disabled copy for quick duplication)
- Inline edit form for endpoints with labeled fields, opens below the row

### Changed
- Uptime calculation now uses raw check entries instead of slot-based aggregation; consistent across all time ranges
- Group uptime = minimum of per-endpoint uptimes (service requires all checks passing)
- Uptime precision increased to 2 decimal places
- Warnings no longer count against uptime (only CRITICAL reduces it)
- Loading spinner is now a full-screen overlay instead of inline (no layout shift)
- Compact vertical layout on status page: tighter header, rows, timeline bars, and footer
- Delete button on Endpoints page uses small inline styling instead of full-width

## 0.1.2

### Added
- TOML configuration file support (default `/etc/kernelci-status.toml`) for port, database path, and default credentials
- Configurable check interval (1–1440 minutes, default 5) and failure retries (0–10, default 3) in admin Configuration page
- Retry logic on probe failure — retries up to N times with 5s delay before marking endpoint as down
- "Change password" button on Users page
- New admin pages: Endpoints (renamed from Configuration), Notifications (email recipients), Configuration (SMTP + scheduler settings)
- "Test Email" button to verify SMTP settings
- HTML-formatted email notifications with status details, timestamps, and KernelCI branding
- Multi-recipient email support (comma-separated addresses with validation)
- SMTP SSL/STARTTLS toggle and From Name field
- AJAX-loaded public status page via HTMX with loading spinner (no full page reloads)
- `/status/data` endpoint for HTMX partial updates (auto-refreshes every 60s)
- Copyright footer on public status page
- Debian packaging: `Dockerfile.bookworm`, `build_deb.sh`, `build_deb_inner.sh`
- systemd service with security hardening (postinst creates user, directory, sets permissions)
- `.dockerignore` and `.gitignore`

### Changed
- Default listen port changed from 8899 to 2001
- CLI flags `--port` and `--db-path` are now optional overrides (config file provides defaults)
- Scheduler reads interval from config cache on every cycle (changes take effect without restart)
- Scheduler uses `tokio::time::sleep` per-cycle instead of fixed `interval` (picks up config changes)
- systemd service uses `--config /etc/kernelci-status.toml` instead of inline flags
- `WorkingDirectory` changed from `/opt/kernelci-status` to `/var/lib/kernelci-status`

### Security
- SMTP password field no longer pre-filled in HTML (shows placeholder, empty submit preserves existing value)
- `GET /api/config` masks `smtp_password` and `api_token` with `********`
- Security headers on all responses: `X-Content-Type-Options`, `X-Frame-Options`, `X-XSS-Protection`, `Referrer-Policy`, `Content-Security-Policy`
- API pagination capped at 1000 rows max
- Sessions invalidated on password change (`DELETE FROM sessions WHERE user_id = ?`)
- History export uses batched writes (5000 rows/batch) instead of loading all into memory
- SQL export escapes all string fields through `quote_sql_literal()`
- URL parameters in history page are percent-encoded to prevent reflected XSS
- Discord webhook errors use generic message (no URL leak)
- Concurrency semaphore (max 20 parallel checks) prevents resource exhaustion
- Per-checker connect timeouts: PostgreSQL (10s connect + 10s query), TLS (10s TCP + 10s handshake), SSH (10s connect + 15s command + 5s close), K8s (15s config + 15s API)
- Latency checker now returns Critical/Warning on HTTP error codes (was always OK)

## 0.1.0

- Initial release
