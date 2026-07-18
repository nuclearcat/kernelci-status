#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-only
# SPDX-FileCopyrightText: 2026 Collabora Ltd.
# Author: Denys Fedoryshchenko <denys.f@collabora.com>

# Build a .deb package for Debian Trixie inside Docker.
# Usage: ./build_deb.sh
# Output: ./output/kernelci-status_<version>_<arch>.deb
set -euo pipefail

cd "$(dirname "$0")"

IMAGE="kernelci-status-builder-trixie"
OUTPUT_DIR="$(pwd)/output"

mkdir -p "${OUTPUT_DIR}"

echo "==> Building Docker image ..."
docker build -f Dockerfile.trixie -t "${IMAGE}" .

echo "==> Building .deb package ..."
docker run --rm -v "${OUTPUT_DIR}:/output" "${IMAGE}"

echo "==> Package written to:"
ls -lh "${OUTPUT_DIR}"/*.deb
