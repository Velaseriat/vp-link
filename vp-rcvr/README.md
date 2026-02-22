# vp-rcvr

HEVC RTP receiver for OBS preview/loopback usage.

## Dependencies

### Build

- Rust toolchain (`cargo`)
- `pkg-config`
- `libdbus-1-dev`

### Runtime

- `gstreamer1.0-tools`
- HEVC decode plugins: `gstreamer1.0-libav` and/or hardware decoder plugins
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
```

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
