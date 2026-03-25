#!/usr/bin/env bash
# Inner build script — runs INSIDE the Docker container.
set -euo pipefail

PKG="kernelci-status"
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
ARCH=$(dpkg-architecture -qDEB_BUILD_ARCH)
DEB_DIR="/build/pkg/${PKG}_${VERSION}_${ARCH}"

echo "==> Building ${PKG} ${VERSION} for ${ARCH} ..."

# ── 1. Compile release binary ──────────────────────────────────────
cargo build --release

# ── 2. Assemble package tree ───────────────────────────────────────
install -Dm755 "target/release/${PKG}" "${DEB_DIR}/usr/local/bin/${PKG}"
install -Dm644 "systemd/${PKG}.service" "${DEB_DIR}/lib/systemd/system/${PKG}.service"
install -Dm640 "${PKG}.toml.example"    "${DEB_DIR}/etc/${PKG}.toml"

# ── 3. DEBIAN control files ────────────────────────────────────────
mkdir -p "${DEB_DIR}/DEBIAN"

cat > "${DEB_DIR}/DEBIAN/control" <<EOF
Package: ${PKG}
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${ARCH}
Depends: libc6
Maintainer: KernelCI <admin@kernelci.org>
Description: KernelCI Status Monitoring Daemon
 Web-based status monitoring dashboard for KernelCI infrastructure.
 Monitors HTTP, Prometheus, PostgreSQL, Kubernetes, and Docker
 endpoints with alerting via Discord, email, and text file.
EOF

cat > "${DEB_DIR}/DEBIAN/conffiles" <<EOF
/etc/${PKG}.toml
EOF

cat > "${DEB_DIR}/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e

# Create system user/group if missing
if ! getent group kernelci-status >/dev/null 2>&1; then
    addgroup --system kernelci-status
fi
if ! getent passwd kernelci-status >/dev/null 2>&1; then
    adduser --system --ingroup kernelci-status \
            --home /var/lib/kernelci-status \
            --no-create-home --shell /usr/sbin/nologin \
            kernelci-status
fi

# Create data directory
install -d -o kernelci-status -g kernelci-status -m 750 /var/lib/kernelci-status

# Protect config file (contains credentials)
chown root:kernelci-status /etc/kernelci-status.toml
chmod 640 /etc/kernelci-status.toml

# Enable and (re)start the service
systemctl daemon-reload
systemctl enable kernelci-status.service
systemctl restart kernelci-status.service || true
POSTINST
chmod 0755 "${DEB_DIR}/DEBIAN/postinst"

cat > "${DEB_DIR}/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
set -e
if [ "$1" = "remove" ] || [ "$1" = "purge" ]; then
    systemctl stop kernelci-status.service || true
    systemctl disable kernelci-status.service || true
fi
PRERM
chmod 0755 "${DEB_DIR}/DEBIAN/prerm"

cat > "${DEB_DIR}/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -e
if [ "$1" = "purge" ]; then
    rm -rf /var/lib/kernelci-status
    deluser --system kernelci-status >/dev/null 2>&1 || true
    delgroup --system kernelci-status >/dev/null 2>&1 || true
fi
systemctl daemon-reload || true
POSTRM
chmod 0755 "${DEB_DIR}/DEBIAN/postrm"

# ── 4. Build .deb ─────────────────────────────────────────────────
fakeroot dpkg-deb --build "${DEB_DIR}"

# Copy artifact to /output (mounted from host)
cp "/build/pkg/${PKG}_${VERSION}_${ARCH}.deb" /output/

echo "==> Done: ${PKG}_${VERSION}_${ARCH}.deb"
