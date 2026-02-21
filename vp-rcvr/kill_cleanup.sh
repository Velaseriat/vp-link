#!/usr/bin/env bash
set -euo pipefail

VIDEO_NR="${VIDEO_NR:-10}"
UNLOAD_LOOPBACK="${UNLOAD_LOOPBACK:-1}"
VIDEO_DEV="/dev/video${VIDEO_NR}"

echo "[vp-link] stopping processes..."
pkill -9 obs 2>/dev/null || true
pkill -f "vp-rcvr.*receive" 2>/dev/null || true
pkill -f "vp-sndr.*send" 2>/dev/null || true
pkill -f "gst-launch-1.0.*(h265|hevc|rtph265pay|rtph265depay|v4l2sink)" 2>/dev/null || true

sleep 1

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
