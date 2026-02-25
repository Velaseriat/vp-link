#!/usr/bin/env bash
set -euo pipefail

VIDEO_NR="${VIDEO_NR:-10}"
PORT="${PORT:-5000}"
CODEC="${CODEC:-h265}"
V4L2_WIDTH="${V4L2_WIDTH:-1600}"
V4L2_HEIGHT="${V4L2_HEIGHT:-900}"
V4L2_FPS="${V4L2_FPS:-60}"
CARD_LABEL="${CARD_LABEL:-vp-link}"
REPO_DIR="${REPO_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
VIDEO_DEV="/dev/video${VIDEO_NR}"
RX_LOG="${RX_LOG:-/tmp/vp-rcvr-receive.log}"
OBS_LOG="${OBS_LOG:-/tmp/vp-rcvr-obs.log}"

echo "[vp-link] stopping OBS and old receiver processes..."
pkill -9 obs 2>/dev/null || true
pkill -f "vp-rcvr.*receive" 2>/dev/null || true
pkill -f "gst-launch.*port=${PORT}" 2>/dev/null || true
sleep 1

echo "[vp-link] reloading v4l2loopback on ${VIDEO_DEV}..."
sudo modprobe -r v4l2loopback 2>/dev/null || true
sudo modprobe v4l2loopback "video_nr=${VIDEO_NR}" "card_label=${CARD_LABEL}" max_buffers=4

if [[ ! -e "${VIDEO_DEV}" ]]; then
  echo "[vp-link] ERROR: ${VIDEO_DEV} not found after modprobe"
  exit 1
fi

echo "[vp-link] priming loopback format (${V4L2_WIDTH}x${V4L2_HEIGHT}@${V4L2_FPS} YU12)..."
v4l2-ctl -d "${VIDEO_DEV}" -c keep_format=0
v4l2-ctl -d "${VIDEO_DEV}" --set-fmt-video="width=${V4L2_WIDTH},height=${V4L2_HEIGHT},pixelformat=YU12" --set-parm="${V4L2_FPS}"
v4l2-ctl -d "${VIDEO_DEV}" -c keep_format=1,sustain_framerate=1,timeout=1000

echo "[vp-link] starting receiver -> ${VIDEO_DEV} (codec ${CODEC}, ${V4L2_WIDTH}x${V4L2_HEIGHT}@${V4L2_FPS}, port ${PORT})..."
cd "${REPO_DIR}"
nohup cargo run --release -- receive --no-preview --codec "${CODEC}" --port "${PORT}" --v4l2-device "${VIDEO_DEV}" --v4l2-width "${V4L2_WIDTH}" --v4l2-height "${V4L2_HEIGHT}" --v4l2-fps "${V4L2_FPS}" >"${RX_LOG}" 2>&1 &
RX_PID=$!
sleep 2

echo "[vp-link] launching OBS..."
nohup obs >"${OBS_LOG}" 2>&1 &
OBS_PID=$!

echo "[vp-link] done"
echo "  receiver pid: ${RX_PID} (log: ${RX_LOG})"
echo "  obs pid:      ${OBS_PID} (log: ${OBS_LOG})"
echo "  video dev:    ${VIDEO_DEV}"
