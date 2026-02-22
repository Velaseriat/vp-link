# vp-sndr

HEVC viewport sender for COSMIC/Wayland.

## Dependencies

### Build

- Rust toolchain (`cargo`)
- `pkg-config`
- `libdbus-1-dev`
- `libgstreamer1.0-dev`
- `libgstreamer-plugins-base1.0-dev`

### Runtime

- `gstreamer1.0-tools`
- `gstreamer1.0-pipewire`
- `gstreamer1.0-libav` and/or hardware codec stack (`gstreamer1.0-vaapi`, etc.)
- `cosmic-screenshot`
- `gdbus`

Install on Pop!_OS/Ubuntu:

```bash
sudo apt update
sudo apt install -y \
  pkg-config libdbus-1-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-tools gstreamer1.0-pipewire \
  gstreamer1.0-libav gstreamer1.0-vaapi gstreamer1.0-plugins-bad
```

## Run

```bash
cd vp-sndr
cargo run --release -- run-saved
```

Service mode using saved config:

```bash
cd vp-sndr
cargo run --release -- run-saved
```

CLI send example:

```bash
cd vp-sndr
cargo run --release -- send \
  --receiver-ip <RECEIVER_IP> --port 5000 \
  --x 200 --y 100 --width 1280 --height 720 \
  --fps 60 --encoder x265enc --bitrate-kbps 8000
```

Show config path:

```bash
cargo run --release -- config
```

## Installed Operation

After running `./install.sh` from the repo root:

- Start sender with:
  - `~/.local/bin/vp-sndr-start.sh`
- Stop sender/receiver with:
  - `~/.local/bin/vp-link-stop.sh`
