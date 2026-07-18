<!--
SPDX-License-Identifier: LGPL-2.1-only
SPDX-FileCopyrightText: 2026 Collabora Ltd.
Author: Denys Fedoryshchenko <denys.f@collabora.com>
-->

# kernelci-status

`kernelci-status` is a small Rust status page and infrastructure monitoring
daemon. It stores state in SQLite, serves an admin UI and public status page,
checks service health on a schedule, and can send notifications for incidents
and maintenance windows.

## Features

- Public status page with endpoint history and uptime summaries
- Admin UI for endpoints, incidents, maintenance windows, notifications, users,
  and reports
- Health checks for HTTP, TLS certificates, Prometheus metrics, PostgreSQL,
  Kubernetes, and Docker-over-SSH
- SQLite storage with automatic schema migration
- Email, Discord webhook, Telegram bot, and text-file notification backends
- Optional ACME/Let's Encrypt TLS support
- Debian package build script

## Build

```sh
cargo build --release
```

The project uses Rust 1.85 or newer.

## Run

```sh
cargo run -- --config kernelci-status.toml.example
```

By default the daemon listens on port `2001` and uses `status.db` in the current
directory unless overridden by configuration or CLI options.

Useful options:

```sh
kernelci-status --config /etc/kernelci-status.toml
kernelci-status --port 2001 --db-path /var/lib/kernelci-status/status.db
kernelci-status create-user admin
kernelci-status passwd admin
```

`passwd` changes an existing user's password and invalidates all of their active sessions. Use
`--config` or `--db-path` to select the correct database when it is not at the default location.

## Configuration

Start from [kernelci-status.toml.example](kernelci-status.toml.example). The
main sections are:

- `[server]` for the HTTP listen port
- `[database]` for the SQLite database path
- `[credentials]` for initial admin user bootstrap
- `[acme]` for automatic Let's Encrypt certificates

Remove the bootstrap credentials after creating the initial user.

## Debian Package

Build a Debian Trixie package inside Docker:

```sh
./build_deb.sh
```

Packages are written to `output/`.

## License

Licensed under the GNU Lesser General Public License version 2.1. See
[LICENSE](LICENSE).
