#!/usr/bin/env bash
set -euo pipefail

PORT="${PORT:-5000}"
REPO_DIR="${REPO_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
RX_LOG="${RX_LOG:-/tmp/vp-rcvr-preview.log}"

echo "[vp-link] stopping old receiver preview processes..."
pkill -f "vp-rcvr.*receive" 2>/dev/null || true
sleep 0.2

echo "[vp-link] starting receiver preview (port ${PORT})..."
cd "${REPO_DIR}"
nohup cargo run --release -- receive --port "${PORT}" >"${RX_LOG}" 2>&1 &
RX_PID=$!

echo "[vp-link] done"
echo "  receiver pid: ${RX_PID} (log: ${RX_LOG})"
