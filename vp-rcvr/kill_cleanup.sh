#!/usr/bin/env bash
set -euo pipefail

PORT="${PORT:-5000}"
VIDEO_NR="${VIDEO_NR:-10}"
UNLOAD_LOOPBACK="${UNLOAD_LOOPBACK:-1}"
VIDEO_DEV="/dev/video${VIDEO_NR}"

echo "[vp-link] stopping processes (port=${PORT})..."
pkill -9 obs 2>/dev/null || true
pkill -f "vp-rcvr.*receive" 2>/dev/null || true
pkill -f "gst-launch.*port=${PORT}" 2>/dev/null || true

sleep 1

# Mop up anything still holding the loopback device
if [[ -e "${VIDEO_DEV}" ]]; then
  sudo fuser -k "${VIDEO_DEV}" 2>/dev/null || true
fi

if [[ "${UNLOAD_LOOPBACK}" == "1" ]]; then
  echo "[vp-link] attempting to unload v4l2loopback..."
  if sudo modprobe -r v4l2loopback 2>/dev/null; then
    echo "[vp-link] unloaded v4l2loopback"
  else
    echo "[vp-link] v4l2loopback still in use or not loaded"
    sudo lsof "${VIDEO_DEV}" 2>/dev/null || true
  fi
fi

echo "[vp-link] cleanup complete"
