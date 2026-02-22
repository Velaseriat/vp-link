#!/usr/bin/env bash
set -euo pipefail

LOG_PATH="${LOG_PATH:-/tmp/vp-sndr.log}"

echo "[vp-link] starting sender with saved config..."
nohup "$HOME/.local/bin/vp-sndr" run-saved >"$LOG_PATH" 2>&1 &
PID=$!
echo "$PID" > /tmp/vp-sndr.pid
echo "[vp-link] sender pid: $PID (log: $LOG_PATH)"
