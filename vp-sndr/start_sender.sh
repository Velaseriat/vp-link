#!/usr/bin/env bash
set -euo pipefail

RECEIVER_IP="${RECEIVER_IP:-10.0.0.11}"
PORT="${PORT:-5000}"
X="${X:-}"
Y="${Y:-}"
WIDTH="${WIDTH:-1280}"
HEIGHT="${HEIGHT:-720}"
FPS="${FPS:-60}"
FOLLOW_MOUSE="${FOLLOW_MOUSE:-1}"
SMOOTHING="${SMOOTHING:-4}"
DEADZONE="${DEADZONE:-25}"
ENCODER="${ENCODER:-x264enc}"
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
  --width "${WIDTH}"
  --height "${HEIGHT}"
  --fps "${FPS}"
  --smoothing "${SMOOTHING}"
  --deadzone "${DEADZONE}"
  --encoder "${ENCODER}"
  --bitrate-kbps "${BITRATE_KBPS}"
)

if [[ -n "${X}" ]]; then
  ARGS+=(--x "${X}")
fi

if [[ -n "${Y}" ]]; then
  ARGS+=(--y "${Y}")
fi

if [[ "${FOLLOW_MOUSE}" == "1" ]]; then
  ARGS+=(--follow-mouse)
fi

nohup cargo "${ARGS[@]}" >"${LOG_PATH}" 2>&1 &
PID=$!
echo "${PID}" >/tmp/vp-sndr.pid

echo "[vp-link] done"
echo "  sender pid: ${PID} (log: ${LOG_PATH})"
