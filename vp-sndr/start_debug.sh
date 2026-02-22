#!/usr/bin/env bash
set -euo pipefail

RECEIVER_IP="${RECEIVER_IP:-127.0.0.1}"
PORT="${PORT:-5000}"
X="${X:-200}"
Y="${Y:-100}"
WIDTH="${WIDTH:-1920}"
HEIGHT="${HEIGHT:-1080}"
FPS="${FPS:-60}"
FOLLOW_MOUSE="${FOLLOW_MOUSE:-1}"
SMOOTHING="${SMOOTHING:-8}"
ENCODER="${ENCODER:-x265enc}"
BITRATE_KBPS="${BITRATE_KBPS:-8000}"
REPO_DIR="${REPO_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
LOG_PATH="${LOG_PATH:-/tmp/vp-sndr-send.log}"

echo "[vp-link] stopping old sender processes..."
pkill -f "vp-sndr.*send" 2>/dev/null || true
sleep 0.2

echo "[vp-link] starting sender -> ${RECEIVER_IP}:${PORT}..."
cd "${REPO_DIR}"

ARGS=(
  run --release -- send
  --receiver-ip "${RECEIVER_IP}"
  --port "${PORT}"
  --x "${X}"
  --y "${Y}"
  --width "${WIDTH}"
  --height "${HEIGHT}"
  --fps "${FPS}"
  --smoothing "${SMOOTHING}"
  --encoder "${ENCODER}"
  --bitrate-kbps "${BITRATE_KBPS}"
)

if [[ "${FOLLOW_MOUSE}" == "1" ]]; then
  ARGS+=(--follow-mouse)
fi

nohup cargo "${ARGS[@]}" >"${LOG_PATH}" 2>&1 &
PID=$!
echo "${PID}" >/tmp/vp-sndr.pid

echo "[vp-link] done"
echo "  sender pid: ${PID} (log: ${LOG_PATH})"
