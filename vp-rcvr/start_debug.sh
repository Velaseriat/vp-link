#!/usr/bin/env bash
set -euo pipefail

PORT="${PORT:-5000}"
CODEC="${CODEC:-h265}"
REPO_DIR="${REPO_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
RX_LOG="${RX_LOG:-/tmp/vp-rcvr-preview.log}"
PREVIEW_WIDTH="${PREVIEW_WIDTH:-}"
PREVIEW_HEIGHT="${PREVIEW_HEIGHT:-}"

echo "[vp-link] stopping old receiver preview processes..."
pkill -f "vp-rcvr.*receive" 2>/dev/null || true
pkill -f "gst-launch.*port=${PORT}" 2>/dev/null || true
sleep 0.3

echo "[vp-link] starting receiver preview (codec ${CODEC}, port ${PORT})..."
cd "${REPO_DIR}"
cmd=(cargo run --release -- receive --codec "${CODEC}" --port "${PORT}")
if [[ -n "${PREVIEW_WIDTH}" ]]; then
  cmd+=(--preview-width "${PREVIEW_WIDTH}")
fi
if [[ -n "${PREVIEW_HEIGHT}" ]]; then
  cmd+=(--preview-height "${PREVIEW_HEIGHT}")
fi

nohup "${cmd[@]}" >"${RX_LOG}" 2>&1 &
RX_PID=$!

echo "[vp-link] done"
echo "  receiver pid: ${RX_PID} (log: ${RX_LOG})"
