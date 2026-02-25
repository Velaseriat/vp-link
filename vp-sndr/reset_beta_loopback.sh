#!/usr/bin/env bash
set -euo pipefail

HOST="${HOST:-beta}"
DEVICE="${DEVICE:-/dev/video10}"
WIDTH="${WIDTH:-1280}"
HEIGHT="${HEIGHT:-720}"
FPS="${FPS:-30}"
PIXEL_FMT="${PIXEL_FMT:-XR24}"
CARD_LABEL="${CARD_LABEL:-vp-link}"
VIDEO_NR="${VIDEO_NR:-10}"
RUN_TEST="${RUN_TEST:-1}"

usage() {
  cat <<'EOF'
Usage: reset_beta_loopback.sh [options]

Options:
  --host HOST        SSH host (default: beta)
  --device PATH      Loopback device path on host (default: /dev/video10)
  --width N          Width (default: 1280)
  --height N         Height (default: 720)
  --fps N            Framerate (default: 30)
  --pixel-fmt FMT    V4L2 pixel format for set-fmt (default: XR24)
  --video-nr N       v4l2loopback video_nr (default: 10)
  --card-label NAME  v4l2loopback card_label (default: vp-link)
  --no-test          Skip local gst-launch write sanity test
  -h, --help         Show this help

Environment overrides also work: HOST, DEVICE, WIDTH, HEIGHT, FPS, PIXEL_FMT, VIDEO_NR, CARD_LABEL, RUN_TEST.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host) HOST="$2"; shift 2 ;;
    --device) DEVICE="$2"; shift 2 ;;
    --width) WIDTH="$2"; shift 2 ;;
    --height) HEIGHT="$2"; shift 2 ;;
    --fps) FPS="$2"; shift 2 ;;
    --pixel-fmt) PIXEL_FMT="$2"; shift 2 ;;
    --video-nr) VIDEO_NR="$2"; shift 2 ;;
    --card-label) CARD_LABEL="$2"; shift 2 ;;
    --no-test) RUN_TEST=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

echo "[beta-loopback] host=${HOST} device=${DEVICE} ${WIDTH}x${HEIGHT}@${FPS} fmt=${PIXEL_FMT}"

echo "[1/6] Show current device mapping..."
ssh "${HOST}" "readlink -f /dev/v4l/by-id/v4l2loopback-vp-link-video || true"

echo "[2/6] List and kill processes using ${DEVICE}..."
ssh -t "${HOST}" "sudo fuser -v ${DEVICE} || true"
ssh -t "${HOST}" "sudo fuser -k ${DEVICE} || true"

echo "[3/6] Reload v4l2loopback module..."
ssh -t "${HOST}" "sudo modprobe -r v4l2loopback && sudo modprobe v4l2loopback video_nr=${VIDEO_NR} card_label=${CARD_LABEL} max_buffers=4"

echo "[4/6] Prime device format/fps (keep_format off first)..."
ssh "${HOST}" "v4l2-ctl -d ${DEVICE} -c keep_format=0"
ssh "${HOST}" "v4l2-ctl -d ${DEVICE} --set-fmt-video=width=${WIDTH},height=${HEIGHT},pixelformat=${PIXEL_FMT} --set-parm=${FPS}"

echo "[5/6] Lock loopback controls for consistency..."
ssh "${HOST}" "v4l2-ctl -d ${DEVICE} -c keep_format=1,sustain_framerate=1,timeout=1000"

echo "[6/6] Final device state..."
ssh "${HOST}" "v4l2-ctl --all -d ${DEVICE}"

if [[ "${RUN_TEST}" == "1" ]]; then
  echo "[test] Local write sanity (gstreamer -> v4l2sink io-mode=rw)..."
  ssh "${HOST}" "timeout 5 gst-launch-1.0 -v videotestsrc is-live=true pattern=smpte ! video/x-raw,format=BGRx,width=${WIDTH},height=${HEIGHT},framerate=${FPS}/1 ! v4l2sink device=${DEVICE} io-mode=rw sync=false"
fi

echo "[beta-loopback] complete"
