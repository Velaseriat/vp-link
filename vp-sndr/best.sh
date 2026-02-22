#!/usr/bin/env bash
set -euo pipefail

exec vp-sndr send \
  --receiver-ip "${RECEIVER_IP:-127.0.0.1}" \
  --port "${PORT:-5000}" \
  --encoder "${ENCODER:-x264enc}" \
  --fps "${FPS:-60}" \
  --follow-mouse \
  --bitrate-kbps "${BITRATE_KBPS:-16000}" \
  --width "${WIDTH:-1920}" \
  --height "${HEIGHT:-1080}"
