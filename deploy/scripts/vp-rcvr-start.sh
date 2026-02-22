#!/usr/bin/env bash
set -euo pipefail

LOG_PATH="${LOG_PATH:-/tmp/vp-rcvr.log}"

"$HOME/.local/bin/vp-rcvr-prestart.sh"

echo "[vp-link] starting receiver with saved config..."
nohup "$HOME/.local/bin/vp-rcvr" run-saved >"$LOG_PATH" 2>&1 &
PID=$!
echo "$PID" > /tmp/vp-rcvr.pid
echo "[vp-link] receiver pid: $PID (log: $LOG_PATH)"
