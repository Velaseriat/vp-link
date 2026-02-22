# vp-link Status

## Current Goal

Build a Wayland/COSMIC-compatible viewport streaming system:
- `vp-sndr`: capture and stream a 1280x720 viewport
- `vp-rcvr`: receive and expose video for OBS source usage

## Current Implementation Progress

### 1. Prototype Projects
- `vp-sndr/` has a Rust sender binary (`vp-sndr`) for HEVC RTP/UDP from portal PipeWire capture.
  - Supports static crop and `--follow-mouse`.
  - Uses the same live crop engine model as `vp-test` (appsink -> CPU crop -> appsrc).
- `vp-rcvr/` now also has a Rust receiver binary (`vp-rcvr`) for HEVC over RTP/UDP.
  - Receives RTP/H265 on UDP and previews via GStreamer sink.
- `vp-test/` exists as the active validation project.

### 2. `vp-test` Commands
- `check`: verifies environment/tool/plugin prerequisites.
- `capture`: validates that `pipewiresrc` can deliver frames.
- `frame`: takes one screenshot and crops to target viewport.
- `record`: records a short cropped `.webm`.
  - Performs ScreenCast DBus handshake (`CreateSession -> SelectSources -> Start`) and extracts PipeWire node id.
  - Uses `pipewiresrc path=<node_id>` on success.
  - `--follow-mouse` now runs a live crop pipeline (`appsink -> CPU crop -> appsrc -> vp8enc`) and uses a deadzone-style follow state machine.

### 3. Verified Behavior
- Screenshot permission prompt works in COSMIC session.
- Portal+PipeWire recording path works and writes `clip.webm`.
- `--follow-mouse` works in live mode, with lerp controlled by `--smoothing`.
- COSMIC cursor session tracker is wired in via vendored `cosmic-client-toolkit` and used as absolute cursor source when metadata is unavailable.
- MVP works end-to-end across sender/receiver:
  - `vp-sndr` sends cropped HEVC RTP stream.
  - `vp-rcvr` receives and displays stream.
  - Aspect ratio and viewport shape are correct.

## Dependencies

### Shared Build Prerequisites (`vp-sndr` + `vp-rcvr`)
- `cargo` / Rust toolchain
- `pkg-config`
- `libdbus-1-dev` (needed by tray dependency via `libdbus-sys`)
- Wayland runtime in COSMIC session

### `vp-sndr` Dependencies
- Build:
  - `libgstreamer1.0-dev`
  - `libgstreamer-plugins-base1.0-dev`
- Runtime:
  - `gstreamer1.0-tools` (`gst-launch-1.0`, `gst-inspect-1.0`, `gst-discoverer-1.0`)
  - `gstreamer1.0-pipewire` (provides `pipewiresrc`)
  - `gstreamer1.0-libav` and/or hardware codec stack (`gstreamer1.0-vaapi`, etc.)
  - `cosmic-screenshot`
  - `gdbus` (portal/service checks)

### `vp-rcvr` Dependencies
- Build:
  - Shared build prerequisites only.
- Runtime:
  - `gstreamer1.0-tools` (`gst-launch-1.0` used by receiver pipeline)
  - HEVC decode plugins: `gstreamer1.0-libav` and/or hardware decoder plugin stack
  - Optional OBS loopback output: `v4l2loopback-dkms` + `v4l2loopback-utils`

### Source Dependencies (In-Repo)
- `vp-test/vendor/cosmic-protocols`
- `cosmic-client-toolkit` as a path dependency from that vendored tree

Install on Pop!_OS/Ubuntu:

```bash
sudo apt update
sudo apt install \
  pkg-config libdbus-1-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-tools gstreamer1.0-pipewire \
  gstreamer1.0-libav gstreamer1.0-vaapi gstreamer1.0-plugins-bad \
  v4l2loopback-dkms v4l2loopback-utils
```

Verify:

```bash
gst-inspect-1.0 pipewiresrc
gst-inspect-1.0 avdec_h265
```

## Known Blockers

- Cursor coordinate alignment still needs real-session validation under multi-monitor / scaling combinations.
- Current follow pipeline does CPU-side RGBA crop; performance tuning is still pending for high FPS + long sessions.
- `vp-sndr`/`vp-rcvr` do not yet expose a direct OBS virtual-camera output path by default (can be added via loopback sink path).

## Next Steps

1. Daily MVP run commands:
   - Receiver: `cd vp-rcvr && cargo run --release -- receive --port 5000`
   - Sender static crop: `cd vp-sndr && cargo run --release --offline -- send --receiver-ip <RECEIVER_IP> --port 5000 --x 200 --y 100 --width 1280 --height 720 --fps 60 --encoder x265enc --bitrate-kbps 8000`
   - Sender mouse-follow: `cd vp-sndr && cargo run --release --offline -- send --receiver-ip <RECEIVER_IP> --port 5000 --x 200 --y 100 --width 1280 --height 720 --fps 60 --follow-mouse --smoothing 8 --encoder x265enc --bitrate-kbps 8000`
2. Add sender telemetry counters (capture fps/output fps/drop counts/encode latency).
3. Add receiver output mode for OBS-friendly loopback source.

## Installable Runtime Model

- `install.sh` now installs:
  - `~/.local/bin/vp-sndr`
  - `~/.local/bin/vp-rcvr`
  - `~/.local/bin/vp-sndr-start.sh`
  - `~/.local/bin/vp-rcvr-start.sh`
  - `~/.local/bin/vp-link-stop.sh`
  - `~/.local/bin/vp-rcvr-prestart.sh`
  - optional OBS helper wrappers:
    - `~/.local/bin/vp-rcvr-start-obs-bridge.sh`
    - `~/.local/bin/vp-link-kill-cleanup.sh`
- Runtime is script-driven (no tray/systemd required).
