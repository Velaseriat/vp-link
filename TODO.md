# TODO

## Performance Optimizations

1. GPU/Hardware crop-convert path
- Replace CPU RGBA crop loop with GPU path where possible (`glupload/glcolorconvert` or VAAPI).
- Keep CPU path as fallback for unsupported systems.

2. Hardware HEVC encode first
- Prefer `vaapih265enc` / `v4l2h265enc`; keep `x265enc` fallback.
- Add encoder capability probe and auto-pick best available encoder.

3. Smarter pacing and backpressure
- Add adaptive frame skip under overload.
- Bound queue sizes and drop oldest work to keep latency low.

4. RTP tuning
- Expose MTU, jitterbuffer latency, keyframe interval, and payloader config.
- Validate packet loss behavior and recovery across LAN/Wi-Fi.

5. Color and format stability
- Lock sender/receiver caps to known-good formats (I420/NV12 where appropriate).
- Verify full-range/limited-range behavior for consistent OBS output.

## Telemetry and Observability

6. Sender metrics
- Capture FPS, output FPS, emit latency, encode latency, frame drop/skip counts.
- Periodic one-line stats log every N seconds.

7. Receiver metrics
- Decode FPS, render FPS, jitterbuffer drops/late packets, v4l2sink throughput.
- Add optional verbose mode for troubleshooting.

8. Structured logging
- Add log levels and concise machine-parseable log mode.
- Include source selection info (stream metadata vs COSMIC cursor vs evdev).

## Reliability and UX

9. Reconnect/recovery behavior
- Auto-recover sender/receiver pipelines on transient failures.
- Retry portal handshake with bounded backoff.

10. Process lifecycle scripts
- Keep `start_obs_bridge.sh` and `kill_cleanup.sh` in repo and documented.
- Add health-check script for sender/receiver + loopback state.

11. Better defaults and profiles
- Add `--profile` presets (`low-latency`, `balanced`, `quality`).
- Allow per-profile encoder and bitrate defaults.

12. OBS integration polish
- Add receiver mode that forces OBS-friendly caps before `v4l2sink`.
- Verify stable visibility with/without `exclusive_caps`.

## Code Cleanup and Refactors

13. Shared crate for common logic
- Extract portal handshake, cursor tracking, and utility code into shared module/crate.
- Remove duplicated logic between `vp-test` and `vp-sndr`.

14. Pipeline builder abstraction
- Replace ad-hoc pipeline strings with typed builder helpers.
- Centralize caps/encoder/payloader configuration.

15. Follow controller module
- Isolate follow state machine (cursor source, lerp, thresholds) into dedicated unit-tested module.
- Add deterministic simulation tests for movement behavior.

16. Error handling cleanup
- Normalize error types and messages (`thiserror` + context).
- Keep user-facing errors actionable and short.

17. Config model
- Introduce a config struct/file (TOML) for runtime settings.
- CLI flags override config values.

18. Remove stale prototype artifacts
- Decide whether to keep/remove Python stubs (`sender.py`, `receiver.py`).
- Keep repo focused on maintained Rust path.

## Testing

19. Basic integration tests
- Smoke test sender->receiver on localhost using test source.
- Validate negotiated caps and successful frame flow.

20. Performance regression checks
- Add repeatable benchmark command for 60/120 FPS scenarios.
- Track CPU, dropped frames, and end-to-end latency over time.

21. Compatibility matrix
- Test on COSMIC versions and common GPU stacks.
- Record known-good encoder/decoder combinations.

## Packaging and Ops

22. Build/release docs
- Add quickstart for sender host and receiver host.
- Include dependency install matrix for Pop!_OS and Ubuntu variants.

23. Service units
- Add optional `systemd --user` units for receiver and sender.
- Include restart policy and startup ordering notes.

24. Versioning and changelog
- Introduce semantic version tags.
- Keep concise changelog entries for behavior-impacting changes.
