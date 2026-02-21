# vp-test

Small Rust probe to answer one question on POP!_OS COSMIC:

"Can this machine acquire desktop video frames through the Wayland/PipeWire path?"

## What it does

- `check`: verifies runtime prerequisites for screencast capture
- `capture`: runs a real `pipewiresrc` pipeline and waits for 120 frames
- `frame`: captures one desktop screenshot and crops a fixed viewport image
- `record`: writes a short cropped `.webm` video

If `capture` succeeds, your environment can provide video frames for a sender app.

## Run

```bash
cd vp-test
cargo run --release -- check
cargo run --release -- capture
cargo run --release -- frame --x 200 --y 100 --out frame-720p.png
cargo run --release -- record --x 200 --y 100 --duration-secs 5 --fps 10 --out clip.webm
```

Optional timeout override:

```bash
cargo run --release -- capture --timeout-secs 20
```

## Notes

- `capture` uses `pipewiresrc num-buffers=120 ... ! fakesink`.
- On many Pop!_OS systems, the `pipewiresrc` plugin comes from `gstreamer1.0-pipewire`.
- If you run from a restricted shell/session without DBus access, `check` may show portal as warning even if your normal desktop session is fine.
- `frame` currently uses `cosmic-screenshot` then GStreamer crop.
- `record` first performs ScreenCast portal handshake (`CreateSession -> SelectSources -> Start`) and uses the returned PipeWire node id with `pipewiresrc`.
- If portal/PipeWire recording fails, `record` falls back to screenshot-sequence mode.
- `record` uses VP8/WebM (`vp8enc` + `webmmux`) to avoid extra codec dependencies.
