#!/usr/bin/env python3
import argparse
import socket
import struct
import time

import cv2
import numpy as np


HEADER_STRUCT = struct.Struct("!IHH")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Viewport receiver: decode UDP chunks and display video for OBS capture.")
    parser.add_argument("--bind-ip", default="0.0.0.0", help="Local IP to bind.")
    parser.add_argument("--port", type=int, default=5000, help="UDP bind port.")
    parser.add_argument("--title", default="vp-rcvr", help="Display window title.")
    parser.add_argument("--stale-seconds", type=float, default=1.0, help="Drop partial frames older than this.")
    parser.add_argument("--fullscreen", action="store_true", help="Start in fullscreen mode.")
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind((args.bind_ip, args.port))
    sock.settimeout(0.2)

    cv2.namedWindow(args.title, cv2.WINDOW_NORMAL)
    cv2.resizeWindow(args.title, 1280, 720)
    if args.fullscreen:
        cv2.setWindowProperty(args.title, cv2.WND_PROP_FULLSCREEN, cv2.WINDOW_FULLSCREEN)

    print(f"Listening on {args.bind_ip}:{args.port}")
    print("Press q or Esc to quit.")

    frames: dict[int, dict] = {}
    last_complete_id = -1
    last_stat_time = time.monotonic()
    shown_frames = 0

    while True:
        now = time.monotonic()
        try:
            packet, _ = sock.recvfrom(65535)
            if len(packet) < HEADER_STRUCT.size:
                continue
            frame_id, total_chunks, chunk_idx = HEADER_STRUCT.unpack(packet[: HEADER_STRUCT.size])
            if chunk_idx >= total_chunks:
                continue

            entry = frames.get(frame_id)
            if entry is None:
                entry = {"total": total_chunks, "chunks": {}, "created": now}
                frames[frame_id] = entry

            if entry["total"] != total_chunks:
                frames.pop(frame_id, None)
                continue

            entry["chunks"][chunk_idx] = packet[HEADER_STRUCT.size :]

            if len(entry["chunks"]) == entry["total"] and frame_id > last_complete_id:
                payload = b"".join(entry["chunks"][i] for i in range(entry["total"]))
                npbuf = np.frombuffer(payload, dtype=np.uint8)
                frame = cv2.imdecode(npbuf, cv2.IMREAD_COLOR)
                frames.pop(frame_id, None)
                if frame is not None:
                    cv2.imshow(args.title, frame)
                    shown_frames += 1
                    last_complete_id = frame_id
        except socket.timeout:
            pass

        cutoff = now - args.stale_seconds
        stale_ids = [fid for fid, entry in frames.items() if entry["created"] < cutoff]
        for fid in stale_ids:
            frames.pop(fid, None)

        key = cv2.waitKey(1) & 0xFF
        if key in (27, ord("q")):
            break

        if now - last_stat_time >= 2.0:
            elapsed = now - last_stat_time
            print(f"display_fps={shown_frames / elapsed:.1f} buffered_frames={len(frames)}")
            shown_frames = 0
            last_stat_time = now

    cv2.destroyAllWindows()


if __name__ == "__main__":
    main()
