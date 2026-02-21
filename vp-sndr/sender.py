#!/usr/bin/env python3
import argparse
import math
import socket
import struct
import time

import cv2
import mss
import numpy as np
from pynput.mouse import Controller as MouseController


HEADER_STRUCT = struct.Struct("!IHH")
DEFAULT_WIDTH = 1280
DEFAULT_HEIGHT = 720


def clamp(value: float, low: float, high: float) -> float:
    return max(low, min(value, high))


def crop_viewport(frame: np.ndarray, center_x: float, center_y: float, width: int, height: int) -> np.ndarray:
    frame_h, frame_w = frame.shape[:2]
    half_w = width // 2
    half_h = height // 2

    left = int(round(center_x)) - half_w
    top = int(round(center_y)) - half_h
    left = max(0, min(left, frame_w - width))
    top = max(0, min(top, frame_h - height))

    return frame[top : top + height, left : left + width]


def send_frame(sock: socket.socket, dest: tuple[str, int], frame_id: int, encoded: bytes, mtu: int) -> None:
    max_payload = max(1, mtu - HEADER_STRUCT.size)
    total_chunks = math.ceil(len(encoded) / max_payload)
    if total_chunks > 65535:
        raise ValueError("Frame is too large for chunk header format.")

    for idx in range(total_chunks):
        start = idx * max_payload
        end = start + max_payload
        payload = encoded[start:end]
        header = HEADER_STRUCT.pack(frame_id, total_chunks, idx)
        sock.sendto(header + payload, dest)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Viewport sender: stream a mouse-following 1280x720 crop over UDP.")
    parser.add_argument("--receiver-ip", required=True, help="IP of vp-rcvr machine.")
    parser.add_argument("--port", type=int, default=5000, help="UDP port on receiver.")
    parser.add_argument("--fps", type=float, default=30.0, help="Capture and send FPS.")
    parser.add_argument("--quality", type=int, default=80, help="JPEG quality (1-100).")
    parser.add_argument("--monitor", type=int, default=1, help="Monitor index from mss (1-based).")
    parser.add_argument("--sample-interval", type=float, default=0.5, help="Mouse target sample interval in seconds.")
    parser.add_argument("--smoothing", type=float, default=8.0, help="Higher value follows target faster.")
    parser.add_argument("--mtu", type=int, default=1400, help="UDP packet size limit.")
    parser.add_argument("--width", type=int, default=DEFAULT_WIDTH, help="Viewport width.")
    parser.add_argument("--height", type=int, default=DEFAULT_HEIGHT, help="Viewport height.")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.fps <= 0:
        raise ValueError("--fps must be > 0")
    if args.sample_interval <= 0:
        raise ValueError("--sample-interval must be > 0")
    if args.width <= 0 or args.height <= 0:
        raise ValueError("--width/--height must be > 0")

    mouse = MouseController()
    dest = (args.receiver_ip, args.port)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

    with mss.mss() as sct:
        if args.monitor < 1 or args.monitor >= len(sct.monitors):
            raise ValueError(f"Invalid --monitor index {args.monitor}. Available: 1..{len(sct.monitors) - 1}")

        monitor = sct.monitors[args.monitor]
        mon_left = monitor["left"]
        mon_top = monitor["top"]
        mon_w = monitor["width"]
        mon_h = monitor["height"]

        if mon_w < args.width or mon_h < args.height:
            raise ValueError(
                f"Monitor {args.monitor} ({mon_w}x{mon_h}) is smaller than viewport {args.width}x{args.height}."
            )

        now = time.monotonic()
        next_sample = now
        mx, my = mouse.position
        target_x = clamp(mx - mon_left, 0, mon_w - 1)
        target_y = clamp(my - mon_top, 0, mon_h - 1)
        center_x = target_x
        center_y = target_y
        frame_id = 0
        frame_interval = 1.0 / args.fps
        last_frame_time = now
        last_stat_time = now
        sent_frames = 0

        print(
            f"Sending {args.width}x{args.height} viewport from monitor {args.monitor} to {args.receiver_ip}:{args.port}"
        )
        print(
            f"Mouse sample interval={args.sample_interval:.3f}s smoothing={args.smoothing:.2f} fps={args.fps:.1f}"
        )

        while True:
            loop_start = time.monotonic()

            if loop_start >= next_sample:
                mx, my = mouse.position
                target_x = clamp(mx - mon_left, 0, mon_w - 1)
                target_y = clamp(my - mon_top, 0, mon_h - 1)
                next_sample = loop_start + args.sample_interval

            dt = max(1e-6, loop_start - last_frame_time)
            last_frame_time = loop_start

            alpha = 1.0 - math.exp(-args.smoothing * dt)
            center_x += (target_x - center_x) * alpha
            center_y += (target_y - center_y) * alpha

            screenshot = sct.grab(monitor)
            frame = np.array(screenshot)[:, :, :3]
            cropped = crop_viewport(frame, center_x, center_y, args.width, args.height)

            ok, encoded = cv2.imencode(".jpg", cropped, [cv2.IMWRITE_JPEG_QUALITY, args.quality])
            if not ok:
                continue

            send_frame(sock, dest, frame_id, encoded.tobytes(), args.mtu)
            frame_id = (frame_id + 1) & 0xFFFFFFFF
            sent_frames += 1

            if loop_start - last_stat_time >= 2.0:
                elapsed = loop_start - last_stat_time
                print(f"send_fps={sent_frames / elapsed:.1f} center=({int(center_x)}, {int(center_y)})")
                sent_frames = 0
                last_stat_time = loop_start

            sleep_time = frame_interval - (time.monotonic() - loop_start)
            if sleep_time > 0:
                time.sleep(sleep_time)


if __name__ == "__main__":
    main()
