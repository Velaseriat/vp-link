# vp-rcvr

H264/H265 RTP receiver for preview and OBS loopback usage.

## Dependencies

### Build

- Rust toolchain (`cargo`)
- `pkg-config`
- `libdbus-1-dev`

### Runtime

- `gstreamer1.0-tools`
- H264/H265 decode plugins: `gstreamer1.0-libav` and/or hardware decoder plugins
- Optional OBS loopback output:
  - `v4l2loopback-dkms`
  - `v4l2loopback-utils`

Install on Pop!_OS/Ubuntu:

```bash
sudo apt update
sudo apt install -y \
  pkg-config libdbus-1-dev \
  gstreamer1.0-tools gstreamer1.0-libav \
  gstreamer1.0-vaapi gstreamer1.0-plugins-bad \
  v4l2loopback-dkms v4l2loopback-utils
```

## Run

```bash
cd vp-rcvr
cargo run --release -- run-saved
```

Service mode using saved config:

```bash
cd vp-rcvr
cargo run --release -- run-saved
```

CLI receive examples:

```bash
cd vp-rcvr
cargo run --release -- receive --port 5000
cargo run --release -- receive --port 5000 --v4l2-device /dev/video10
cargo run --release -- receive --port 5000 --no-preview --v4l2-device /dev/video10
cargo run --release -- receive --codec h264 --port 5000 --no-preview --v4l2-device /dev/video10 --v4l2-width 1280 --v4l2-height 720 --v4l2-fps 60
```

V4L2 loopback output caps are optional and can be forced when OBS has trouble opening the device at the default mode:

- `--v4l2-width`
- `--v4l2-height`
- `--v4l2-fps`

Show config path:

```bash
cargo run --release -- config
```

## Installed Operation

After running `./install.sh` from the repo root:

- Start receiver with:
  - `~/.local/bin/vp-rcvr-start.sh`
- Stop sender/receiver with:
  - `~/.local/bin/vp-link-stop.sh`
- Receiver start script runs pre-start cleanup/setup (`~/.local/bin/vp-rcvr-prestart.sh`) that:
  - clears stale receiver/gst processes
  - reads `v4l2_device` from `~/.config/vp-link/vp-rcvr.toml`
  - ensures the matching `/dev/videoN` loopback exists (loads `v4l2loopback` if needed)

Auto loopback creation requires passwordless sudo for `modprobe` in user services:

```bash
sudo visudo
```

Add a rule like:

```text
<your-user> ALL=(root) NOPASSWD: /usr/sbin/modprobe
```

## OBS Bridge Script

`start_obs_bridge.sh` launches `vp-rcvr` in OBS loopback mode with:

- `--no-preview` (no extra preview window)
- `exclusive_caps=1` on `v4l2loopback`
- configurable loopback mode via env vars:
  - `CODEC` (`h264` or `h265`)
  - `V4L2_WIDTH`
  - `V4L2_HEIGHT`
  - `V4L2_FPS`

Example:

```bash
cd vp-rcvr
CODEC=h264 V4L2_WIDTH=1280 V4L2_HEIGHT=720 V4L2_FPS=60 ./start_obs_bridge.sh
```
