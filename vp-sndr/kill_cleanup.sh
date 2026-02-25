#!/usr/bin/env bash
set -euo pipefail

PORT="${PORT:-5000}"

echo "[vp-link] stopping sender processes (port=${PORT})..."
pkill -f "vp-sndr.*send" 2>/dev/null || true
pkill -f "vp-sndr.*run-saved" 2>/dev/null || true
pkill -f "gst-launch.*port=${PORT}" 2>/dev/null || true

sleep 0.5

rm -f /tmp/vp-sndr.pid

echo "[vp-link] cleanup complete"
