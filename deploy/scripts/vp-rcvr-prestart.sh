#!/usr/bin/env bash
set -euo pipefail

# Best-effort cleanup before starting receiver service.
pkill -f "vp-rcvr.*receive" 2>/dev/null || true
pkill -f "gst-launch-1.0.*rtph265depay.*v4l2sink" 2>/dev/null || true
sleep 0.2

CFG_PATH="${XDG_CONFIG_HOME:-$HOME/.config}/vp-link/vp-rcvr.toml"
VIDEO_DEV="${VIDEO_DEV:-}"
CARD_LABEL="${CARD_LABEL:-vp-link}"
EXCLUSIVE_CAPS="${EXCLUSIVE_CAPS:-0}"

# Resolve loopback target from saved receiver config when not provided explicitly.
if [[ -z "${VIDEO_DEV}" && -f "${CFG_PATH}" ]]; then
  VIDEO_DEV="$(sed -n 's/^v4l2_device = "\(.*\)"/\1/p' "${CFG_PATH}" | head -n1 || true)"
fi

# If receiver is not configured for v4l2 output, no loopback setup is needed.
if [[ -z "${VIDEO_DEV}" ]]; then
  exit 0
fi

if [[ "${VIDEO_DEV}" =~ ^/dev/video([0-9]+)$ ]]; then
  VIDEO_NR="${VIDEO_NR:-${BASH_REMATCH[1]}}"
else
  echo "[vp-link] ERROR: unsupported v4l2_device path: ${VIDEO_DEV}"
  exit 1
fi

if [[ ! -e "${VIDEO_DEV}" ]]; then
  echo "[vp-link] ${VIDEO_DEV} missing, attempting v4l2loopback setup..."

  if ! command -v modprobe >/dev/null 2>&1; then
    echo "[vp-link] ERROR: modprobe not found"
    exit 1
  fi

  if ! sudo -n true >/dev/null 2>&1; then
    echo "[vp-link] ERROR: passwordless sudo is required to auto-load v4l2loopback"
    echo "[vp-link] Hint: run install manually once:"
    echo "  sudo modprobe v4l2loopback video_nr=${VIDEO_NR} card_label=${CARD_LABEL} exclusive_caps=${EXCLUSIVE_CAPS}"
    exit 1
  fi

  sudo -n modprobe -r v4l2loopback 2>/dev/null || true
  sudo -n modprobe v4l2loopback video_nr="${VIDEO_NR}" card_label="${CARD_LABEL}" exclusive_caps="${EXCLUSIVE_CAPS}"
fi

if [[ ! -e "${VIDEO_DEV}" ]]; then
  echo "[vp-link] ERROR: ${VIDEO_DEV} is still missing after loopback setup"
  exit 1
fi
