#!/usr/bin/env bash
set -euo pipefail

echo "[vp-link] stopping sender/related processes..."
pkill -f "vp-sndr.*send" 2>/dev/null || true
pkill -f "vp-sndr.*run-saved" 2>/dev/null || true
pkill -f "gst-launch-1.0.*(h265|hevc|rtph265pay|rtph265depay)" 2>/dev/null || true

rm -f /tmp/vp-sndr.pid

echo "[vp-link] cleanup complete"
