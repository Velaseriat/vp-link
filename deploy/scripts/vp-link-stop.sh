#!/usr/bin/env bash
set -euo pipefail

echo "[vp-link] stopping sender/receiver..."
pkill -f "$HOME/.local/bin/vp-sndr run-saved" 2>/dev/null || true
pkill -f "$HOME/.local/bin/vp-rcvr run-saved" 2>/dev/null || true
pkill -f "vp-sndr.*send" 2>/dev/null || true
pkill -f "vp-rcvr.*receive" 2>/dev/null || true

rm -f /tmp/vp-sndr.pid /tmp/vp-rcvr.pid
echo "[vp-link] stopped"
