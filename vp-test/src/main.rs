use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
use ashpd::desktop::PersistMode;
use cosmic_client_toolkit::screencopy::{
    CaptureCursorSession, CaptureFrame, CaptureSession, CaptureSource, FailureReason, Formats,
    Frame, ScreencopyCursorSessionData, ScreencopyCursorSessionDataExt, ScreencopyHandler,
    ScreencopyState,
};
use cosmic_client_toolkit::sctk;
use cosmic_client_toolkit::sctk::output::{OutputHandler, OutputState};
use cosmic_client_toolkit::sctk::registry::{ProvidesRegistryState, RegistryState};
use cosmic_client_toolkit::sctk::seat::pointer::{PointerEvent, PointerHandler};
use cosmic_client_toolkit::sctk::seat::{Capability, SeatHandler, SeatState};
use cosmic_client_toolkit::wayland_client::globals::registry_queue_init as wl_registry_queue_init;
use cosmic_client_toolkit::wayland_client::protocol::{wl_buffer, wl_output, wl_pointer, wl_seat};
use cosmic_client_toolkit::wayland_client::{Connection as WlConnection, QueueHandle as WlQueueHandle, WEnum};
use cosmic_client_toolkit::{delegate_screencopy, wayland_client::delegate_noop};
use evdev::{Device, EventSummary, EventType, RelativeAxisCode};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSinkCallbacks, AppSrc};
use std::collections::VecDeque;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_CAPTURE_TIMEOUT_SECS: u64 = 12;
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const PORTAL_TIMEOUT_SECS: u64 = 15;
const DEFAULT_MOUSE_SAMPLE_INTERVAL_SECS: f64 = 0.5;
const DEFAULT_MOUSE_SMOOTHING: f64 = 8.0;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match parse_cli(&args) {
        Ok(Cli::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(Cli::Check) => run_check(),
        Ok(Cli::Capture { timeout_secs }) => run_capture(timeout_secs),
        Ok(Cli::Frame {
            x,
            y,
            width,
            height,
            out,
        }) => run_frame(x, y, width, height, &out),
        Ok(Cli::Record {
            x,
            y,
            width,
            height,
            duration_secs,
            fps,
            frame_skip,
            out,
            follow_mouse,
            sample_interval_secs,
            smoothing,
        }) => run_record(
            x,
            y,
            width,
            height,
            duration_secs,
            fps,
            frame_skip,
            &out,
            follow_mouse,
            sample_interval_secs,
            smoothing,
        ),
        Err(err) => {
            eprintln!("error: {err}");
            print_help();
            ExitCode::from(2)
        }
    }
}

enum Cli {
    Help,
    Check,
    Capture { timeout_secs: u64 },
    Frame {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        out: PathBuf,
    },
    Record {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        duration_secs: u32,
        fps: u32,
        frame_skip: u32,
        out: PathBuf,
        follow_mouse: bool,
        sample_interval_secs: f64,
        smoothing: f64,
    },
}

fn parse_cli(args: &[String]) -> Result<Cli, String> {
    if args.len() <= 1 {
        return Ok(Cli::Help);
    }

    match args[1].as_str() {
        "-h" | "--help" | "help" => Ok(Cli::Help),
        "check" => Ok(Cli::Check),
        "capture" => {
            let mut timeout_secs = DEFAULT_CAPTURE_TIMEOUT_SECS;
            let mut i = 2usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--timeout-secs" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --timeout-secs".to_string())?;
                        timeout_secs = next
                            .parse::<u64>()
                            .map_err(|_| format!("invalid --timeout-secs value: {next}"))?;
                        i += 2;
                    }
                    unknown => return Err(format!("unknown argument: {unknown}")),
                }
            }
            Ok(Cli::Capture { timeout_secs })
        }
        "frame" => {
            let mut x = 0u32;
            let mut y = 0u32;
            let mut width = DEFAULT_WIDTH;
            let mut height = DEFAULT_HEIGHT;
            let mut out = PathBuf::from("vp-frame.png");

            let mut i = 2usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--x" => {
                        let next = args.get(i + 1).ok_or_else(|| "missing value after --x".to_string())?;
                        x = next.parse::<u32>().map_err(|_| format!("invalid --x value: {next}"))?;
                        i += 2;
                    }
                    "--y" => {
                        let next = args.get(i + 1).ok_or_else(|| "missing value after --y".to_string())?;
                        y = next.parse::<u32>().map_err(|_| format!("invalid --y value: {next}"))?;
                        i += 2;
                    }
                    "--width" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --width".to_string())?;
                        width = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --width value: {next}"))?;
                        i += 2;
                    }
                    "--height" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --height".to_string())?;
                        height = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --height value: {next}"))?;
                        i += 2;
                    }
                    "--out" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --out".to_string())?;
                        out = PathBuf::from(next);
                        i += 2;
                    }
                    unknown => return Err(format!("unknown argument: {unknown}")),
                }
            }

            if width == 0 || height == 0 {
                return Err("--width and --height must be > 0".to_string());
            }

            Ok(Cli::Frame {
                x,
                y,
                width,
                height,
                out,
            })
        }
        "record" => {
            let mut x = 0u32;
            let mut y = 0u32;
            let mut width = DEFAULT_WIDTH;
            let mut height = DEFAULT_HEIGHT;
            let mut duration_secs = 5u32;
            let mut fps = 10u32;
            let mut frame_skip = 0u32;
            let mut out = PathBuf::from("vp-record.webm");
            let mut follow_mouse = false;
            let mut sample_interval_secs = DEFAULT_MOUSE_SAMPLE_INTERVAL_SECS;
            let mut smoothing = DEFAULT_MOUSE_SMOOTHING;

            let mut i = 2usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--x" => {
                        let next = args.get(i + 1).ok_or_else(|| "missing value after --x".to_string())?;
                        x = next.parse::<u32>().map_err(|_| format!("invalid --x value: {next}"))?;
                        i += 2;
                    }
                    "--y" => {
                        let next = args.get(i + 1).ok_or_else(|| "missing value after --y".to_string())?;
                        y = next.parse::<u32>().map_err(|_| format!("invalid --y value: {next}"))?;
                        i += 2;
                    }
                    "--width" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --width".to_string())?;
                        width = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --width value: {next}"))?;
                        i += 2;
                    }
                    "--height" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --height".to_string())?;
                        height = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --height value: {next}"))?;
                        i += 2;
                    }
                    "--duration-secs" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --duration-secs".to_string())?;
                        duration_secs = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --duration-secs value: {next}"))?;
                        i += 2;
                    }
                    "--fps" => {
                        let next = args.get(i + 1).ok_or_else(|| "missing value after --fps".to_string())?;
                        fps = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --fps value: {next}"))?;
                        i += 2;
                    }
                    "--frame-skip" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --frame-skip".to_string())?;
                        frame_skip = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --frame-skip value: {next}"))?;
                        i += 2;
                    }
                    "--out" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --out".to_string())?;
                        out = PathBuf::from(next);
                        i += 2;
                    }
                    "--follow-mouse" => {
                        follow_mouse = true;
                        i += 1;
                    }
                    "--sample-interval" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --sample-interval".to_string())?;
                        sample_interval_secs = next
                            .parse::<f64>()
                            .map_err(|_| format!("invalid --sample-interval value: {next}"))?;
                        i += 2;
                    }
                    "--smoothing" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --smoothing".to_string())?;
                        smoothing = next
                            .parse::<f64>()
                            .map_err(|_| format!("invalid --smoothing value: {next}"))?;
                        i += 2;
                    }
                    unknown => return Err(format!("unknown argument: {unknown}")),
                }
            }

            if width == 0 || height == 0 {
                return Err("--width and --height must be > 0".to_string());
            }
            if duration_secs == 0 {
                return Err("--duration-secs must be > 0".to_string());
            }
            if fps == 0 {
                return Err("--fps must be > 0".to_string());
            }
            if sample_interval_secs <= 0.0 {
                return Err("--sample-interval must be > 0".to_string());
            }
            if smoothing <= 0.0 {
                return Err("--smoothing must be > 0".to_string());
            }

            Ok(Cli::Record {
                x,
                y,
                width,
                height,
                duration_secs,
                fps,
                frame_skip,
                out,
                follow_mouse,
                sample_interval_secs,
                smoothing,
            })
        }
        unknown => Err(format!("unknown command: {unknown}")),
    }
}

fn run_check() -> ExitCode {
    let mut failures = 0u32;

    println!("== Session ==");
    let xdg_session_type = env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "<unset>".to_string());
    let xdg_current_desktop =
        env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "<unset>".to_string());
    let wayland_display = env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "<unset>".to_string());
    println!("XDG_SESSION_TYPE={xdg_session_type}");
    println!("XDG_CURRENT_DESKTOP={xdg_current_desktop}");
    println!("WAYLAND_DISPLAY={wayland_display}");
    if xdg_session_type != "wayland" {
        println!("FAIL: Not in a Wayland session.");
        failures += 1;
    } else {
        println!("PASS: Wayland session detected.");
    }

    println!("\n== Tools ==");
    failures += (!check_command_exists("gst-launch-1.0")).into_u32();
    failures += (!check_command_exists("gst-inspect-1.0")).into_u32();
    failures += (!check_command_exists("gst-discoverer-1.0")).into_u32();
    failures += (!check_command_exists("gdbus")).into_u32();
    failures += (!check_command_exists("cosmic-screenshot")).into_u32();

    println!("\n== GStreamer Plugins ==");
    if check_gst_plugin("pipewiresrc") {
        println!("PASS: pipewiresrc plugin is installed.");
    } else {
        println!("FAIL: pipewiresrc plugin is missing.");
        println!("Hint: On Pop!_OS/Ubuntu this is often provided by package `gstreamer1.0-pipewire`.");
        failures += 1;
    }

    println!("\n== Portal Service (best effort) ==");
    match Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.DBus",
            "--object-path",
            "/org/freedesktop/DBus",
            "--method",
            "org.freedesktop.DBus.NameHasOwner",
            "org.freedesktop.portal.Desktop",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.contains("true") {
                println!("PASS: org.freedesktop.portal.Desktop is active.");
            } else {
                println!("FAIL: org.freedesktop.portal.Desktop is not active.");
                failures += 1;
            }
        }
        Ok(out) => {
            println!(
                "WARN: Could not query DBus session bus (exit {}).",
                out.status.code().unwrap_or(-1)
            );
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.trim().is_empty() {
                println!("dbus stderr: {}", err.trim());
            }
        }
        Err(err) => {
            println!("WARN: Could not invoke gdbus: {err}");
        }
    }

    println!("\n== Result ==");
    if failures == 0 {
        println!("PASS: Basic capture prerequisites look good.");
        println!("Next: run `cargo run --release -- capture` to attempt real frame capture.");
        ExitCode::SUCCESS
    } else {
        println!("FAIL: {failures} prerequisite checks failed.");
        ExitCode::from(1)
    }
}

fn run_capture(timeout_secs: u64) -> ExitCode {
    println!("Running capture probe with timeout={timeout_secs}s");
    if !check_gst_plugin("pipewiresrc") {
        eprintln!("pipewiresrc is missing. Run `cargo run -- check` for details.");
        return ExitCode::from(1);
    }

    // num-buffers forces the pipeline to exit only after receiving real frames.
    // If no frames arrive, we hit timeout and fail the probe.
    let mut child = match Command::new("gst-launch-1.0")
        .args([
            "-q",
            "pipewiresrc",
            "num-buffers=120",
            "do-timestamp=true",
            "!",
            "videoconvert",
            "!",
            "video/x-raw,framerate=30/1",
            "!",
            "fakesink",
            "sync=false",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            eprintln!("Failed to start gst-launch-1.0: {err}");
            return ExitCode::from(1);
        }
    };

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    println!("PASS: Received 120 frames from pipewiresrc.");
                    return ExitCode::SUCCESS;
                }
                let stderr = child
                    .wait_with_output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stderr).to_string())
                    .unwrap_or_default();
                eprintln!(
                    "FAIL: gst-launch exited with code {}.",
                    status.code().unwrap_or(-1)
                );
                if !stderr.trim().is_empty() {
                    eprintln!("gstreamer stderr: {}", stderr.trim());
                }
                return ExitCode::from(1);
            }
            Ok(None) => {
                if start.elapsed() >= Duration::from_secs(timeout_secs) {
                    let _ = child.kill();
                    let output = child.wait_with_output().ok();
                    eprintln!("FAIL: Timed out waiting for frames.");
                    if let Some(out) = output {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        if !stderr.trim().is_empty() {
                            eprintln!("gstreamer stderr: {}", stderr.trim());
                        }
                    }
                    return ExitCode::from(1);
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(err) => {
                eprintln!("FAIL: Error while waiting for gst-launch: {err}");
                let _ = child.kill();
                return ExitCode::from(1);
            }
        }
    }
}

fn run_frame(x: u32, y: u32, width: u32, height: u32, out: &Path) -> ExitCode {
    println!("Capturing single screenshot via cosmic-screenshot...");
    let tmp = unique_temp_dir();
    if let Err(err) = fs::create_dir_all(&tmp) {
        eprintln!("FAIL: could not create temp dir {}: {err}", tmp.display());
        return ExitCode::from(1);
    }

    let shot_path = match capture_screenshot(&tmp) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("FAIL: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return ExitCode::from(1);
        }
    };

    let (img_w, img_h) = match discover_image_dimensions(&shot_path) {
        Some(dims) => dims,
        None => {
            eprintln!(
                "FAIL: could not determine dimensions for screenshot {}",
                shot_path.display()
            );
            let _ = fs::remove_dir_all(&tmp);
            return ExitCode::from(1);
        }
    };
    if img_w < width || img_h < height {
        eprintln!(
            "FAIL: source screenshot is {}x{}, smaller than requested crop {}x{}",
            img_w, img_h, width, height
        );
        let _ = fs::remove_dir_all(&tmp);
        return ExitCode::from(1);
    }

    let max_x = img_w - width;
    let max_y = img_h - height;
    let clamped_x = x.min(max_x);
    let clamped_y = y.min(max_y);
    let right = img_w - (clamped_x + width);
    let bottom = img_h - (clamped_y + height);

    let crop_status = Command::new("gst-launch-1.0")
        .args([
            "-q",
            "filesrc",
            &format!("location={}", shot_path.display()),
            "!",
            "decodebin",
            "!",
            "videoconvert",
            "!",
            "videocrop",
            &format!("left={clamped_x}"),
            &format!("right={right}"),
            &format!("top={clamped_y}"),
            &format!("bottom={bottom}"),
            "!",
            &format!("video/x-raw,width={width},height={height}"),
            "!",
            "pngenc",
            "!",
            "filesink",
            &format!("location={}", out.display()),
        ])
        .status();

    match crop_status {
        Ok(status) if status.success() => {}
        Ok(status) => {
            eprintln!(
                "FAIL: crop pipeline exited with code {}",
                status.code().unwrap_or(-1)
            );
            let _ = fs::remove_dir_all(&tmp);
            return ExitCode::from(1);
        }
        Err(err) => {
            eprintln!("FAIL: could not run crop pipeline: {err}");
            let _ = fs::remove_dir_all(&tmp);
            return ExitCode::from(1);
        }
    }

    println!(
        "PASS: wrote {}x{} frame to {} (source {}x{}, crop x={}, y={})",
        width,
        height,
        out.display(),
        img_w,
        img_h,
        clamped_x,
        clamped_y
    );
    let _ = fs::remove_dir_all(&tmp);
    ExitCode::SUCCESS
}

fn run_record(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    duration_secs: u32,
    fps: u32,
    frame_skip: u32,
    out: &Path,
    follow_mouse: bool,
    sample_interval_secs: f64,
    smoothing: f64,
) -> ExitCode {
    let frames = duration_secs.saturating_mul(fps);
    if frames == 0 {
        eprintln!("FAIL: frame count is zero.");
        return ExitCode::from(1);
    }
    let keep_every = frame_skip.saturating_add(1);
    let mut output_fps = fps / keep_every;
    if output_fps == 0 {
        output_fps = 1;
    }
    if fps % keep_every != 0 {
        eprintln!(
            "WARN: output fps rounded down to {} from {}/{}.",
            output_fps, fps, keep_every
        );
    }
    println!(
        "Recording {}s at capture_fps={} output_fps={} (capture_frames={} keep_every={}), crop {}x{} at x={}, y={}",
        duration_secs, fps, output_fps, frames, keep_every, width, height, x, y
    );
    if follow_mouse {
        println!(
            "Mouse follow enabled (sample_interval={}s, smoothing={}).",
            sample_interval_secs, smoothing
        );
    }

    if !check_gst_plugin("pipewiresrc") {
        eprintln!("FAIL: pipewiresrc plugin missing.");
        return ExitCode::from(1);
    }

    println!("Using PipeWire recording path via portal ScreenCast handshake.");
    match start_portal_screencast() {
        Ok(sc) => {
            println!("Portal stream node id: {}", sc.node_id);
            if follow_mouse {
                return run_record_follow_live(
                    sc.node_id,
                    x,
                    y,
                    width,
                    height,
                    frames,
                    fps,
                    output_fps,
                    frame_skip,
                    out,
                    sample_interval_secs,
                    smoothing,
                );
            }
            let status = Command::new("gst-launch-1.0")
                .args([
                    "-e",
                    "-q",
                    "pipewiresrc",
                    &format!("path={}", sc.node_id),
                    &format!("num-buffers={frames}"),
                    "do-timestamp=true",
                    "!",
                    "videoconvert",
                    "!",
                    "videoscale",
                    "!",
                    "videorate",
                    "drop-only=true",
                    &format!("max-rate={output_fps}"),
                    "!",
                    "videocrop",
                    &format!("left={x}"),
                    &format!("right=0"),
                    &format!("top={y}"),
                    &format!("bottom=0"),
                    "!",
                    &format!("video/x-raw,width={width},height={height},framerate={output_fps}/1"),
                    "!",
                    "vp8enc",
                    "deadline=1",
                    "cpu-used=8",
                    "end-usage=cbr",
                    "target-bitrate=4000000",
                    "!",
                    "webmmux",
                    "!",
                    "filesink",
                    &format!("location={}", out.display()),
                ])
                .status();
            match status {
                Ok(s) if s.success() => {
                    println!("PASS: wrote recording to {}", out.display());
                    ExitCode::SUCCESS
                }
                Ok(s) => {
                    eprintln!("FAIL: pipewire recording pipeline exited with code {}", s.code().unwrap_or(-1));
                    ExitCode::from(1)
                }
                Err(err) => {
                    eprintln!("FAIL: could not run pipewire recording pipeline: {err}");
                    ExitCode::from(1)
                }
            }
        }
        Err(err) => {
            eprintln!("FAIL: portal ScreenCast handshake failed: {err}");
            ExitCode::from(1)
        }
    }
}

#[derive(Clone, Copy)]
struct FollowState {
    center_x: f64,
    center_y: f64,
    cursor_x: f64,
    cursor_y: f64,
    target_x: f64,
    target_y: f64,
    follow_active: bool,
    next_sample_at: Instant,
    last_frame_at: Instant,
}

fn run_record_follow_live(
    node_id: u32,
    x: u32,
    y: u32,
    out_w: u32,
    out_h: u32,
    frames: u32,
    capture_fps: u32,
    output_fps: u32,
    frame_skip: u32,
    out: &Path,
    sample_interval_secs: f64,
    smoothing: f64,
) -> ExitCode {
    if let Err(err) = gst::init() {
        eprintln!("FAIL: gstreamer init failed: {err}");
        return ExitCode::from(1);
    }

    let input_desc = format!(
        "pipewiresrc path={} do-timestamp=true num-buffers={} ! videoconvert ! video/x-raw,format=RGBA ! appsink name=sink max-buffers=1 drop=true emit-signals=true sync=false",
        node_id, frames
    );
    let output_desc = format!(
        "appsrc name=src is-live=true format=time do-timestamp=true block=true caps=video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! videoconvert ! vp8enc deadline=1 cpu-used=8 end-usage=cbr target-bitrate=4000000 ! webmmux ! filesink location={}",
        out_w,
        out_h,
        output_fps,
        out.display()
    );

    let input_pipeline = match gst::parse::launch(&input_desc) {
        Ok(p) => match p.downcast::<gst::Pipeline>() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("FAIL: input pipeline is not a gst::Pipeline");
                return ExitCode::from(1);
            }
        },
        Err(err) => {
            eprintln!("FAIL: could not build input pipeline: {err}");
            return ExitCode::from(1);
        }
    };
    let output_pipeline = match gst::parse::launch(&output_desc) {
        Ok(p) => match p.downcast::<gst::Pipeline>() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("FAIL: output pipeline is not a gst::Pipeline");
                return ExitCode::from(1);
            }
        },
        Err(err) => {
            eprintln!("FAIL: could not build output pipeline: {err}");
            return ExitCode::from(1);
        }
    };

    let appsink = match input_pipeline.by_name("sink").and_then(|e| e.downcast::<AppSink>().ok()) {
        Some(v) => v,
        None => {
            eprintln!("FAIL: could not find appsink in input pipeline");
            return ExitCode::from(1);
        }
    };
    let appsrc = match output_pipeline.by_name("src").and_then(|e| e.downcast::<AppSrc>().ok()) {
        Some(v) => v,
        None => {
            eprintln!("FAIL: could not find appsrc in output pipeline");
            return ExitCode::from(1);
        }
    };

    let cosmic_cursor = match start_cosmic_cursor_tracker() {
        Ok(v) => {
            eprintln!("INFO: COSMIC cursor tracker started.");
            Some(v)
        }
        Err(err) => {
            eprintln!("WARN: COSMIC cursor tracker unavailable: {err}");
            None
        }
    };
    let mouse_deltas = match start_mouse_delta_tracker() {
        Ok(v) => Some(v),
        Err(err) => {
            eprintln!("WARN: evdev mouse delta fallback unavailable: {err}");
            None
        }
    };
    let saw_mouse_delta = Arc::new(AtomicBool::new(false));
    let saw_meta_cursor = Arc::new(AtomicBool::new(false));
    let saw_cosmic_cursor = Arc::new(AtomicBool::new(false));
    let logged_meta_probe = Arc::new(AtomicBool::new(false));

    let follow_state = Arc::new(Mutex::new(FollowState {
        center_x: x as f64 + out_w as f64 / 2.0,
        center_y: y as f64 + out_h as f64 / 2.0,
        cursor_x: x as f64 + out_w as f64 / 2.0,
        cursor_y: y as f64 + out_h as f64 / 2.0,
        target_x: x as f64 + out_w as f64 / 2.0,
        target_y: y as f64 + out_h as f64 / 2.0,
        follow_active: false,
        next_sample_at: Instant::now(),
        last_frame_at: Instant::now(),
    }));

    let frame_count = Arc::new(Mutex::new(0u64));
    let input_frame_count = Arc::new(Mutex::new(0u64));
    let follow_state_cb = Arc::clone(&follow_state);
    let mouse_deltas_cb = mouse_deltas.clone();
    let cosmic_cursor_cb = cosmic_cursor.clone();
    let saw_mouse_delta_cb = Arc::clone(&saw_mouse_delta);
    let saw_meta_cursor_cb = Arc::clone(&saw_meta_cursor);
    let saw_cosmic_cursor_cb = Arc::clone(&saw_cosmic_cursor);
    let logged_meta_probe_cb = Arc::clone(&logged_meta_probe);
    let frame_count_cb = Arc::clone(&frame_count);
    let input_frame_count_cb = Arc::clone(&input_frame_count);
    let appsrc_cb = appsrc.clone();

    appsink.set_callbacks(
        AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                let s = caps.structure(0).ok_or(gst::FlowError::Error)?;
                let src_w = s.get::<i32>("width").map_err(|_| gst::FlowError::Error)? as usize;
                let src_h = s.get::<i32>("height").map_err(|_| gst::FlowError::Error)? as usize;
                let out_w_us = out_w as usize;
                let out_h_us = out_h as usize;
                if src_w < out_w_us || src_h < out_h_us {
                    return Err(gst::FlowError::Error);
                }

                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let src = map.as_slice();
                let src_stride = src_w * 4;

                let now = Instant::now();
                let (crop_x, crop_y) = {
                    let mut st = follow_state_cb.lock().map_err(|_| gst::FlowError::Error)?;
                    let prev_cursor_x = st.cursor_x;
                    let prev_cursor_y = st.cursor_y;
                    let mut used_meta_cursor = false;
                    if let Some((mx, my)) =
                        extract_cursor_from_sample(&sample, src_w as u32, src_h as u32)
                    {
                        st.cursor_x = mx;
                        st.cursor_y = my;
                        used_meta_cursor = true;
                        saw_meta_cursor_cb.store(true, Ordering::Relaxed);
                    } else if let Some(cosmic_cursor_xy) = &cosmic_cursor_cb {
                        let mut used_cosmic = false;
                        if let Ok(guard) = cosmic_cursor_xy.lock() {
                            if let Some((mx, my)) = *guard {
                                st.cursor_x = mx;
                                st.cursor_y = my;
                                saw_cosmic_cursor_cb.store(true, Ordering::Relaxed);
                                used_cosmic = true;
                            }
                        }
                        if !used_cosmic {
                            if let Some(deltas_arc) = &mouse_deltas_cb {
                                let mut deltas =
                                    deltas_arc.lock().map_err(|_| gst::FlowError::Error)?;
                                st.cursor_x += deltas.0;
                                st.cursor_y += deltas.1;
                                if deltas.0.abs() > 0.0 || deltas.1.abs() > 0.0 {
                                    saw_mouse_delta_cb.store(true, Ordering::Relaxed);
                                }
                                deltas.0 = 0.0;
                                deltas.1 = 0.0;
                            }
                        }
                    } else {
                        if let Some(deltas_arc) = &mouse_deltas_cb {
                            let mut deltas = deltas_arc.lock().map_err(|_| gst::FlowError::Error)?;
                            st.cursor_x += deltas.0;
                            st.cursor_y += deltas.1;
                            if deltas.0.abs() > 0.0 || deltas.1.abs() > 0.0 {
                                saw_mouse_delta_cb.store(true, Ordering::Relaxed);
                            }
                            deltas.0 = 0.0;
                            deltas.1 = 0.0;
                        }
                    }

                    let max_cursor_x = (src_w.saturating_sub(1)) as f64;
                    let max_cursor_y = (src_h.saturating_sub(1)) as f64;
                    st.cursor_x = st.cursor_x.clamp(0.0, max_cursor_x);
                    st.cursor_y = st.cursor_y.clamp(0.0, max_cursor_y);
                    let cursor_moved =
                        (st.cursor_x - prev_cursor_x).abs() > 0.001 || (st.cursor_y - prev_cursor_y).abs() > 0.001;

                    if !logged_meta_probe_cb.swap(true, Ordering::Relaxed) {
                        log_sample_meta_once(&sample, used_meta_cursor);
                    }

                    let left = (st.center_x - out_w as f64 / 2.0).clamp(0.0, (src_w - out_w_us) as f64);
                    let top = (st.center_y - out_h as f64 / 2.0).clamp(0.0, (src_h - out_h_us) as f64);
                    let right = left + out_w as f64;
                    let bottom = top + out_h as f64;
                    let in_bounds = st.cursor_x >= left
                        && st.cursor_x < right
                        && st.cursor_y >= top
                        && st.cursor_y < bottom;

                    let prev_follow = st.follow_active;
                    st.follow_active = !in_bounds;
                    if !st.follow_active {
                        st.target_x = st.center_x;
                        st.target_y = st.center_y;
                    } else if cursor_moved || !prev_follow {
                        // Retarget immediately when the cursor moves while outside the deadzone.
                        st.target_x = st.cursor_x;
                        st.target_y = st.cursor_y;
                    }

                    if prev_follow != st.follow_active {
                        eprintln!(
                            "follow_state={} cursor=({:.1},{:.1}) bounds=({:.1},{:.1})-({:.1},{:.1})",
                            if st.follow_active { "ON" } else { "OFF" },
                            st.cursor_x,
                            st.cursor_y,
                            left,
                            top,
                            right,
                            bottom
                        );
                        st.next_sample_at = now + Duration::from_secs_f64(sample_interval_secs);
                    } else if now >= st.next_sample_at {
                        eprintln!(
                            "follow_tick state={} cursor=({:.1},{:.1}) bounds=({:.1},{:.1})-({:.1},{:.1})",
                            if st.follow_active { "ON" } else { "OFF" },
                            st.cursor_x,
                            st.cursor_y,
                            left,
                            top,
                            right,
                            bottom
                        );
                        st.next_sample_at = now + Duration::from_secs_f64(sample_interval_secs);
                    }
                    let dt = (now - st.last_frame_at).as_secs_f64().max(0.000_001);
                    st.last_frame_at = now;
                    let alpha = 1.0 - (-smoothing * dt).exp();
                    st.center_x += (st.target_x - st.center_x) * alpha;
                    st.center_y += (st.target_y - st.center_y) * alpha;
                    let max_x = (src_w - out_w_us) as f64;
                    let max_y = (src_h - out_h_us) as f64;
                    let x = (st.center_x - out_w as f64 / 2.0).clamp(0.0, max_x).round() as usize;
                    let y = (st.center_y - out_h as f64 / 2.0).clamp(0.0, max_y).round() as usize;
                    (x, y)
                };

                let should_emit = {
                    let mut c = input_frame_count_cb
                        .lock()
                        .map_err(|_| gst::FlowError::Error)?;
                    let idx = *c;
                    *c += 1;
                    idx % u64::from(frame_skip.saturating_add(1)) == 0
                };
                if !should_emit {
                    return Ok(gst::FlowSuccess::Ok);
                }

                let mut out_data = vec![0u8; out_w_us * out_h_us * 4];
                for row in 0..out_h_us {
                    let src_off = (crop_y + row) * src_stride + crop_x * 4;
                    let dst_off = row * out_w_us * 4;
                    out_data[dst_off..dst_off + out_w_us * 4]
                        .copy_from_slice(&src[src_off..src_off + out_w_us * 4]);
                }

                let mut out_buf = gst::Buffer::from_mut_slice(out_data);
                {
                    let idx = {
                        let mut c = frame_count_cb.lock().map_err(|_| gst::FlowError::Error)?;
                        let v = *c;
                        *c += 1;
                        v
                    };
                    let dur = gst::ClockTime::from_nseconds(1_000_000_000u64 / output_fps as u64);
                    let pts =
                        gst::ClockTime::from_nseconds((1_000_000_000u64 * idx) / output_fps as u64);
                    let b = out_buf.get_mut().ok_or(gst::FlowError::Error)?;
                    b.set_pts(pts);
                    b.set_duration(dur);
                }

                appsrc_cb.push_buffer(out_buf).map_err(|_| gst::FlowError::Error)?;
                Ok(gst::FlowSuccess::Ok)
            })
            .eos(move |_| {
                let _ = appsrc.end_of_stream();
            })
            .build(),
    );

    if output_pipeline.set_state(gst::State::Playing).is_err() {
        eprintln!("FAIL: could not set output pipeline to Playing");
        return ExitCode::from(1);
    }
    if input_pipeline.set_state(gst::State::Playing).is_err() {
        let _ = output_pipeline.set_state(gst::State::Null);
        eprintln!("FAIL: could not set input pipeline to Playing");
        return ExitCode::from(1);
    }

    let out_bus = match output_pipeline.bus() {
        Some(v) => v,
        None => {
            let _ = input_pipeline.set_state(gst::State::Null);
            let _ = output_pipeline.set_state(gst::State::Null);
            eprintln!("FAIL: could not get output bus");
            return ExitCode::from(1);
        }
    };
    let in_bus = match input_pipeline.bus() {
        Some(v) => v,
        None => {
            let _ = input_pipeline.set_state(gst::State::Null);
            let _ = output_pipeline.set_state(gst::State::Null);
            eprintln!("FAIL: could not get input bus");
            return ExitCode::from(1);
        }
    };

    let deadline =
        Instant::now() + Duration::from_secs((frames as f64 / capture_fps as f64).ceil() as u64 + 20);
    let mut finished = false;
    while Instant::now() < deadline {
        if let Some(msg) = out_bus.timed_pop(gst::ClockTime::from_mseconds(100)) {
            match msg.view() {
                gst::MessageView::Eos(..) => {
                    finished = true;
                    break;
                }
                gst::MessageView::Error(e) => {
                    eprintln!(
                        "FAIL: output pipeline error from {}: {}",
                        e.src().map(|s| s.path_string()).unwrap_or_else(|| "<unknown>".into()),
                        e.error()
                    );
                    break;
                }
                _ => {}
            }
        }
        if let Some(msg) = in_bus.timed_pop(gst::ClockTime::from_mseconds(0)) {
            if let gst::MessageView::Error(e) = msg.view() {
                eprintln!(
                    "FAIL: input pipeline error from {}: {}",
                    e.src().map(|s| s.path_string()).unwrap_or_else(|| "<unknown>".into()),
                    e.error()
                );
                break;
            }
        }
    }

    let _ = input_pipeline.set_state(gst::State::Null);
    let _ = output_pipeline.set_state(gst::State::Null);
    if saw_meta_cursor.load(Ordering::Relaxed) {
        eprintln!("INFO: cursor metadata was detected and used.");
    } else if saw_cosmic_cursor.load(Ordering::Relaxed) {
        eprintln!("INFO: using COSMIC cursor session coordinates.");
    } else {
        eprintln!("INFO: no usable cursor metadata detected; using evdev delta fallback.");
    }
    if mouse_deltas.is_some() && !saw_mouse_delta.load(Ordering::Relaxed) {
        eprintln!("WARN: no mouse delta events were captured from /dev/input during recording.");
    }
    if finished {
        println!("PASS: wrote recording to {}", out.display());
        ExitCode::SUCCESS
    } else {
        eprintln!("FAIL: live follow pipeline timed out before EOS");
        ExitCode::from(1)
    }
}

fn extract_cursor_from_sample(sample: &gst::Sample, src_w: u32, src_h: u32) -> Option<(f64, f64)> {
    let buffer = sample.buffer()?;
    for meta in buffer.iter_meta::<gst::Meta>() {
        if let Some(custom) = meta.try_as_custom_meta() {
            let st = custom.structure();
            let name = st.name().as_str().to_ascii_lowercase();
            let looks_cursor = name.contains("cursor") || name.contains("pointer");
            if !looks_cursor {
                continue;
            }

            if let Some((x, y)) = read_xy_from_structure(st, src_w, src_h) {
                return Some((x, y));
            }
        }
    }
    None
}

fn read_xy_from_structure(st: &gst::StructureRef, src_w: u32, src_h: u32) -> Option<(f64, f64)> {
    let x_num = st.get::<f64>("x").ok().or_else(|| st.get::<i32>("x").ok().map(|v| v as f64))?;
    let y_num = st.get::<f64>("y").ok().or_else(|| st.get::<i32>("y").ok().map(|v| v as f64))?;
    if x_num >= 0.0 && y_num >= 0.0 && x_num <= src_w as f64 && y_num <= src_h as f64 {
        Some((x_num, y_num))
    } else if x_num >= 0.0 && y_num >= 0.0 && x_num <= 1.0 && y_num <= 1.0 {
        Some((x_num * src_w as f64, y_num * src_h as f64))
    } else {
        None
    }
}

fn log_sample_meta_once(sample: &gst::Sample, used_meta_cursor: bool) {
    if let Some(buffer) = sample.buffer() {
        let mut parts: Vec<String> = Vec::new();
        for meta in buffer.iter_meta::<gst::Meta>() {
            let mut label = meta.api().name().to_string();
            if let Some(custom) = meta.try_as_custom_meta() {
                label.push_str(&format!(" custom:{}", custom.structure().name()));
            }
            parts.push(label);
        }
        eprintln!(
            "meta_probe used_meta_cursor={} metas=[{}]",
            used_meta_cursor,
            parts.join(", ")
        );
    }
}

fn unique_temp_dir() -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis();
    env::temp_dir().join(format!("vp-test-{ts}"))
}

fn latest_image_in_dir(dir: &Path) -> Option<PathBuf> {
    let mut latest: Option<(SystemTime, PathBuf)> = None;
    let read_dir = fs::read_dir(dir).ok()?;
    for entry in read_dir {
        let entry = entry.ok()?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        if !matches!(ext.as_deref(), Some("png" | "jpg" | "jpeg" | "bmp" | "webp")) {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(UNIX_EPOCH);

        match &latest {
            None => latest = Some((modified, path)),
            Some((current, _)) if modified > *current => latest = Some((modified, path)),
            _ => {}
        }
    }
    latest.map(|(_, path)| path)
}

fn capture_screenshot(dir: &Path) -> Result<PathBuf, String> {
    let screenshot_status = Command::new("cosmic-screenshot")
        .args([
            "--interactive=false",
            "--modal=false",
            "--notify=false",
            "--save-dir",
        ])
        .arg(dir)
        .stdout(Stdio::null())
        .status()
        .map_err(|e| format!("failed to run cosmic-screenshot: {e}"))?;
    if !screenshot_status.success() {
        return Err(format!(
            "cosmic-screenshot exited with code {}",
            screenshot_status.code().unwrap_or(-1)
        ));
    }
    latest_image_in_dir(dir).ok_or_else(|| format!("no screenshot file found in {}", dir.display()))
}

fn discover_image_dimensions(path: &Path) -> Option<(u32, u32)> {
    let out = Command::new("gst-discoverer-1.0")
        .arg(path.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(v) = trimmed.strip_prefix("Width:") {
            width = v.trim().split(' ').next()?.parse::<u32>().ok();
        }
        if let Some(v) = trimmed.strip_prefix("Height:") {
            height = v.trim().split(' ').next()?.parse::<u32>().ok();
        }
    }

    match (width, height) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => None,
    }
}

#[derive(Default)]
struct CursorSessionData {
    cursor_session_data: ScreencopyCursorSessionData,
}

impl ScreencopyCursorSessionDataExt for CursorSessionData {
    fn screencopy_cursor_session_data(&self) -> &ScreencopyCursorSessionData {
        &self.cursor_session_data
    }
}

struct CosmicCursorApp {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    screencopy_state: ScreencopyState,
    pointer: Option<wl_pointer::WlPointer>,
    cursor_session: Option<CaptureCursorSession>,
    cursor_xy: Arc<Mutex<Option<(f64, f64)>>>,
    logged_first_cursor: bool,
}

impl ProvidesRegistryState for CosmicCursorApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    sctk::registry_handlers!(OutputState, SeatState);
}

impl OutputHandler for CosmicCursorApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl SeatHandler for CosmicCursorApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &WlConnection, _: &WlQueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &WlConnection,
        qh: &WlQueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            if let Ok(pointer) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(pointer);
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &WlConnection, _: &WlQueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for CosmicCursorApp {
    fn pointer_frame(
        &mut self,
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        _events: &[PointerEvent],
    ) {
    }
}

impl ScreencopyHandler for CosmicCursorApp {
    fn screencopy_state(&mut self) -> &mut ScreencopyState {
        &mut self.screencopy_state
    }

    fn init_done(
        &mut self,
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
        _session: &CaptureSession,
        _formats: &Formats,
    ) {
    }

    fn stopped(
        &mut self,
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
        _session: &CaptureSession,
    ) {
    }

    fn ready(
        &mut self,
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
        _screencopy_frame: &CaptureFrame,
        _frame: Frame,
    ) {
    }

    fn failed(
        &mut self,
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
        _screencopy_frame: &CaptureFrame,
        _reason: WEnum<FailureReason>,
    ) {
    }

    fn cursor_position(
        &mut self,
        _conn: &WlConnection,
        _qh: &WlQueueHandle<Self>,
        _cursor_session: &CaptureCursorSession,
        x: i32,
        y: i32,
    ) {
        if let Ok(mut cursor_xy) = self.cursor_xy.lock() {
            *cursor_xy = Some((x as f64, y as f64));
        }
        if !self.logged_first_cursor {
            eprintln!("INFO: first COSMIC cursor event at ({x},{y})");
            self.logged_first_cursor = true;
        }
    }
}

fn start_cosmic_cursor_tracker() -> Result<Arc<Mutex<Option<(f64, f64)>>>, String> {
    let cursor_xy = Arc::new(Mutex::new(None));
    let cursor_xy_thread = Arc::clone(&cursor_xy);
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

    thread::spawn(move || {
        if let Err(err) = run_cosmic_cursor_tracker_loop(cursor_xy_thread, ready_tx.clone()) {
            let _ = ready_tx.send(Err(err));
        }
    });

    match ready_rx.recv_timeout(Duration::from_secs(4)) {
        Ok(Ok(())) => Ok(cursor_xy),
        Ok(Err(err)) => Err(err),
        Err(_) => Err("timed out initializing COSMIC cursor tracker".to_string()),
    }
}

fn run_cosmic_cursor_tracker_loop(
    cursor_xy: Arc<Mutex<Option<(f64, f64)>>>,
    ready_tx: mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    let conn = WlConnection::connect_to_env()
        .map_err(|e| format!("wayland connect failed for cursor tracker: {e}"))?;
    let (globals, mut event_queue) =
        wl_registry_queue_init(&conn).map_err(|e| format!("wayland registry init failed: {e}"))?;
    let qh = event_queue.handle();

    let mut app = CosmicCursorApp {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        screencopy_state: ScreencopyState::new(&globals, &qh),
        pointer: None,
        cursor_session: None,
        cursor_xy,
        logged_first_cursor: false,
    };

    event_queue
        .roundtrip(&mut app)
        .map_err(|e| format!("initial wayland roundtrip failed: {e}"))?;

    let output = app
        .output_state
        .outputs()
        .next()
        .ok_or_else(|| "no wl_output available for cursor tracker".to_string())?;

    let wait_deadline = Instant::now() + Duration::from_secs(3);
    while app.pointer.is_none() && Instant::now() < wait_deadline {
        event_queue
            .blocking_dispatch(&mut app)
            .map_err(|e| format!("waiting for pointer capability failed: {e}"))?;
    }
    let pointer = app
        .pointer
        .clone()
        .ok_or_else(|| "no wl_pointer capability available for cursor tracker".to_string())?;

    let cursor_session = app
        .screencopy_state
        .capturer()
        .create_cursor_session(
            &CaptureSource::Output(output),
            &pointer,
            &qh,
            CursorSessionData::default(),
        )
        .map_err(|e| format!("create_cursor_session failed: {e}"))?;
    app.cursor_session = Some(cursor_session);
    let _ = ready_tx.send(Ok(()));

    loop {
        event_queue
            .blocking_dispatch(&mut app)
            .map_err(|e| format!("cursor tracker dispatch failed: {e}"))?;
    }
}

sctk::delegate_registry!(CosmicCursorApp);
sctk::delegate_output!(CosmicCursorApp);
sctk::delegate_seat!(CosmicCursorApp);
sctk::delegate_pointer!(CosmicCursorApp);
delegate_screencopy!(CosmicCursorApp);
delegate_noop!(CosmicCursorApp: ignore wl_buffer::WlBuffer);

fn start_mouse_delta_tracker() -> Result<Arc<Mutex<(f64, f64)>>, String> {
    let mut devices: VecDeque<Device> = VecDeque::new();
    let entries = fs::read_dir("/dev/input").map_err(|e| format!("failed to scan /dev/input: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("event") {
            continue;
        }
        if let Ok(dev) = Device::open(&path) {
            let has_relative = dev.supported_events().contains(EventType::RELATIVE);
            if has_relative {
                let _ = dev.set_nonblocking(true);
                devices.push_back(dev);
            }
        }
    }
    if devices.is_empty() {
        return Err("no relative mouse devices found in /dev/input/event*".to_string());
    }

    let deltas = Arc::new(Mutex::new((0.0f64, 0.0f64)));
    let deltas_thread = Arc::clone(&deltas);
    thread::spawn(move || loop {
        for dev in &mut devices {
            match dev.fetch_events() {
                Ok(events) => {
                    for ev in events {
                        match ev.destructure() {
                            EventSummary::RelativeAxis(_, RelativeAxisCode::REL_X, v) => {
                                if let Ok(mut d) = deltas_thread.lock() {
                                    d.0 += v as f64;
                                }
                            }
                            EventSummary::RelativeAxis(_, RelativeAxisCode::REL_Y, v) => {
                                if let Ok(mut d) = deltas_thread.lock() {
                                    d.1 += v as f64;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Err(_) => {}
            }
        }
        thread::sleep(Duration::from_millis(2));
    });
    Ok(deltas)
}

struct PortalScreenCast {
    node_id: u32,
}

fn start_portal_screencast() -> Result<PortalScreenCast, String> {
    println!("Portal: CreateSession...");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create tokio runtime: {e}"))?;

    rt.block_on(async {
        let portal = Screencast::new()
            .await
            .map_err(|e| format!("failed to connect to ScreenCast portal: {e}"))?;

        let session = tokio::time::timeout(
            Duration::from_secs(PORTAL_TIMEOUT_SECS),
            portal.create_session(),
        )
        .await
        .map_err(|_| "CreateSession timed out".to_string())?
        .map_err(|e| format!("CreateSession failed: {e}"))?;

        let available_cursor_modes = portal
            .available_cursor_modes()
            .await
            .map_err(|e| format!("Failed to query available cursor modes: {e}"))?;
        let cursor_mode = if available_cursor_modes.contains(CursorMode::Metadata) {
            CursorMode::Metadata
        } else if available_cursor_modes.contains(CursorMode::Embedded) {
            CursorMode::Embedded
        } else {
            CursorMode::Hidden
        };

        println!("Portal: SelectSources...");
        tokio::time::timeout(
            Duration::from_secs(PORTAL_TIMEOUT_SECS),
            portal.select_sources(
                &session,
                cursor_mode,
                SourceType::Monitor.into(),
                false,
                None,
                PersistMode::DoNot,
            ),
        )
        .await
        .map_err(|_| "SelectSources timed out".to_string())?
        .map_err(|e| format!("SelectSources failed: {e}"))?;

        println!("Portal: Start (watch for COSMIC picker popup)...");
        let request = tokio::time::timeout(
            Duration::from_secs(PORTAL_TIMEOUT_SECS),
            portal.start(&session, None),
        )
        .await
        .map_err(|_| "Start timed out".to_string())?
        .map_err(|e| format!("Start failed: {e}"))?;
        let response = request
            .response()
            .map_err(|e| format!("Start response failed: {e}"))?;

        let streams = response.streams();
        let stream = streams
            .first()
            .ok_or_else(|| "Start returned no streams".to_string())?;
        Ok(PortalScreenCast {
            node_id: stream.pipe_wire_node_id(),
        })
    })
}

fn check_command_exists(cmd: &str) -> bool {
    let exists = Command::new("which")
        .arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if exists {
        println!("PASS: found command `{cmd}`.");
    } else {
        println!("FAIL: missing command `{cmd}`.");
    }
    exists
}

fn check_gst_plugin(plugin: &str) -> bool {
    Command::new("gst-inspect-1.0")
        .arg(OsStr::new(plugin))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

trait BoolToU32 {
    fn into_u32(self) -> u32;
}

impl BoolToU32 for bool {
    fn into_u32(self) -> u32 {
        if self { 1 } else { 0 }
    }
}

fn print_help() {
    println!("vp-test: COSMIC/Wayland screencast probe");
    println!();
    println!("Usage:");
    println!("  vp-test check");
    println!("  vp-test capture [--timeout-secs N]");
    println!("  vp-test frame [--x N] [--y N] [--width N] [--height N] [--out PATH]");
    println!("  vp-test record [--x N] [--y N] [--width N] [--height N] [--duration-secs N] [--fps N] [--frame-skip N] [--out PATH] [--follow-mouse] [--sample-interval S] [--smoothing K]");
    println!();
    println!("Commands:");
    println!("  check      Validate session, tools, pipewire plugin, and portal presence.");
    println!("  capture    Attempt to pull 120 frames from pipewiresrc.");
    println!("  frame      Capture one screenshot and crop a viewport frame.");
    println!("  record     Record a short cropped video (.webm), using PipeWire when available.");
}
