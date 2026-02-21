# vp-sync Status

## Current Goal

Build a Wayland/COSMIC-compatible viewport streaming system:
- `vp-sndr`: capture and stream a 1280x720 viewport
- `vp-rcvr`: receive and expose video for OBS source usage

## Current Implementation Progress

### 1. Prototype Projects
- `vp-sndr/` and `vp-rcvr/` exist (early Python prototype).
- `vp-test/` exists as the active validation project.

### 2. `vp-test` Commands
- `check`: verifies environment/tool/plugin prerequisites.
- `capture`: validates that `pipewiresrc` can deliver frames.
- `frame`: takes one screenshot and crops to target viewport.
- `record`: records a short cropped `.webm`.
  - Performs ScreenCast DBus handshake (`CreateSession -> SelectSources -> Start`) and extracts PipeWire node id.
  - Uses `pipewiresrc path=<node_id>` on success.
  - Falls back to screenshot-sequence path on handshake or pipeline failure.

### 3. Verified Behavior
- Screenshot permission prompt works in COSMIC session.
- Fallback record path works but is low FPS (not suitable for real streaming).
- `pipewiresrc` can still fail with `target not found` unless portal handshake is performed first.

## Dependencies

### Required (Current)
- `cargo` / Rust toolchain
- `gstreamer1.0-tools` (`gst-launch-1.0`, `gst-inspect-1.0`, `gst-discoverer-1.0`)
- `cosmic-screenshot`
- `gdbus` (for portal/service checks)

### Required for Real-Time Video Capture
- `gstreamer1.0-pipewire` (provides `pipewiresrc`)

Install on Pop!_OS/Ubuntu:

```bash
sudo apt update
sudo apt install gstreamer1.0-tools gstreamer1.0-pipewire
```

Verify:

```bash
gst-inspect-1.0 pipewiresrc
```

## Known Blockers

- Screenshot fallback cannot maintain target FPS for streaming.
- Portal handshake parsing is currently implemented via `gdbus monitor` text parsing (works for testing, but should be replaced with native DBus client for production robustness).

## Next Steps

1. Re-run:
   - `cd vp-test && cargo run --release -- check`
   - `cd vp-test && cargo run --release -- capture`
   - `cd vp-test && cargo run --release -- record --duration-secs 5 --fps 30 --out clip.webm`
2. If PipeWire path succeeds consistently, move from `vp-test` into Rust sender/receiver implementation.
