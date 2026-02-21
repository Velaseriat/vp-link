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
use cosmic_client_toolkit::wayland_client::{
    Connection as WlConnection, QueueHandle as WlQueueHandle, WEnum,
};
use cosmic_client_toolkit::{delegate_screencopy, wayland_client::delegate_noop};
use evdev::{Device, EventSummary, EventType, RelativeAxisCode};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSinkCallbacks, AppSrc};
use std::collections::VecDeque;
use std::env;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const PORTAL_TIMEOUT_SECS: u64 = 15;
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const DEFAULT_MOUSE_SAMPLE_INTERVAL_SECS: f64 = 0.5;
const DEFAULT_MOUSE_SMOOTHING: f64 = 8.0;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match parse_cli(&args) {
        Ok(Cli::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(Cli::Send {
            receiver_ip,
            port,
            x,
            y,
            width,
            height,
            fps,
            frame_skip,
            follow_mouse,
            sample_interval_secs,
            smoothing,
            encoder,
            bitrate_kbps,
        }) => run_send(SendCfg {
            receiver_ip,
            port,
            x,
            y,
            width,
            height,
            fps,
            frame_skip,
            follow_mouse,
            sample_interval_secs,
            smoothing,
            encoder,
            bitrate_kbps,
        }),
        Err(err) => {
            eprintln!("error: {err}");
            print_help();
            ExitCode::from(2)
        }
    }
}

enum Cli {
    Help,
    Send {
        receiver_ip: String,
        port: u16,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        fps: u32,
        frame_skip: u32,
        follow_mouse: bool,
        sample_interval_secs: f64,
        smoothing: f64,
        encoder: String,
        bitrate_kbps: u32,
    },
}

struct SendCfg {
    receiver_ip: String,
    port: u16,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    fps: u32,
    frame_skip: u32,
    follow_mouse: bool,
    sample_interval_secs: f64,
    smoothing: f64,
    encoder: String,
    bitrate_kbps: u32,
}

fn parse_cli(args: &[String]) -> Result<Cli, String> {
    if args.len() <= 1 {
        return Ok(Cli::Help);
    }
    match args[1].as_str() {
        "-h" | "--help" | "help" => Ok(Cli::Help),
        "send" => {
            let mut receiver_ip: Option<String> = None;
            let mut port = 5000u16;
            let mut x = 0u32;
            let mut y = 0u32;
            let mut width = DEFAULT_WIDTH;
            let mut height = DEFAULT_HEIGHT;
            let mut fps = 60u32;
            let mut frame_skip = 0u32;
            let mut follow_mouse = false;
            let mut sample_interval_secs = DEFAULT_MOUSE_SAMPLE_INTERVAL_SECS;
            let mut smoothing = DEFAULT_MOUSE_SMOOTHING;
            let mut encoder = String::from("x265enc");
            let mut bitrate_kbps = 8000u32;

            let mut i = 2usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--receiver-ip" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --receiver-ip".to_string())?;
                        receiver_ip = Some(next.clone());
                        i += 2;
                    }
                    "--port" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --port".to_string())?;
                        port = next
                            .parse::<u16>()
                            .map_err(|_| format!("invalid --port value: {next}"))?;
                        i += 2;
                    }
                    "--x" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --x".to_string())?;
                        x = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --x value: {next}"))?;
                        i += 2;
                    }
                    "--y" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --y".to_string())?;
                        y = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --y value: {next}"))?;
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
                    "--fps" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --fps".to_string())?;
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
                    "--encoder" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --encoder".to_string())?;
                        encoder = next.clone();
                        i += 2;
                    }
                    "--bitrate-kbps" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --bitrate-kbps".to_string())?;
                        bitrate_kbps = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --bitrate-kbps value: {next}"))?;
                        i += 2;
                    }
                    other => return Err(format!("unknown argument: {other}")),
                }
            }
            let receiver_ip =
                receiver_ip.ok_or_else(|| "missing required argument --receiver-ip".to_string())?;
            if width == 0 || height == 0 {
                return Err("--width and --height must be > 0".to_string());
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
            if bitrate_kbps == 0 {
                return Err("--bitrate-kbps must be > 0".to_string());
            }

            Ok(Cli::Send {
                receiver_ip,
                port,
                x,
                y,
                width,
                height,
                fps,
                frame_skip,
                follow_mouse,
                sample_interval_secs,
                smoothing,
                encoder,
                bitrate_kbps,
            })
        }
        other => Err(format!("unknown command: {other}")),
    }
}

fn run_send(cfg: SendCfg) -> ExitCode {
    let keep_every = cfg.frame_skip.saturating_add(1);
    let mut output_fps = cfg.fps / keep_every;
    if output_fps == 0 {
        output_fps = 1;
    }
    println!(
        "Sending to {}:{} capture_fps={} output_fps={} keep_every={} crop={}x{} at x={}, y={}",
        cfg.receiver_ip,
        cfg.port,
        cfg.fps,
        output_fps,
        keep_every,
        cfg.width,
        cfg.height,
        cfg.x,
        cfg.y
    );
    if cfg.follow_mouse {
        println!(
            "Mouse follow enabled (sample_interval={}s, smoothing={}).",
            cfg.sample_interval_secs, cfg.smoothing
        );
    }
    let sc = match start_portal_screencast() {
        Ok(v) => v,
        Err(err) => {
            eprintln!("FAIL: portal ScreenCast handshake failed: {err}");
            return ExitCode::from(1);
        }
    };
    println!("Portal stream node id: {}", sc.node_id);

    run_send_live(sc.node_id, cfg, output_fps)
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

fn encoder_stage(encoder: &str, fps: u32, bitrate_kbps: u32) -> Result<String, String> {
    match encoder {
        "x265enc" => Ok(format!(
            "x265enc tune=zerolatency speed-preset=ultrafast key-int-max={} bitrate={}",
            fps.max(1),
            bitrate_kbps
        )),
        "vaapih265enc" => Ok(format!(
            "vaapih265enc rate-control=cbr bitrate={} keyframe-period={}",
            bitrate_kbps,
            fps.max(1)
        )),
        "v4l2h265enc" => Ok(format!(
            "v4l2h265enc extra-controls=\"controls,video_bitrate={}000\"",
            bitrate_kbps
        )),
        other => Err(format!("unsupported --encoder '{other}'")),
    }
}

fn run_send_live(node_id: u32, cfg: SendCfg, output_fps: u32) -> ExitCode {
    if let Err(err) = gst::init() {
        eprintln!("FAIL: gstreamer init failed: {err}");
        return ExitCode::from(1);
    }

    let enc = match encoder_stage(&cfg.encoder, output_fps, cfg.bitrate_kbps) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("FAIL: {err}");
            return ExitCode::from(2);
        }
    };

    let input_desc = format!(
        "pipewiresrc path={} do-timestamp=true ! videoconvert ! video/x-raw,format=RGBA,framerate={}/1 ! appsink name=sink max-buffers=1 drop=true emit-signals=true sync=false",
        node_id, cfg.fps
    );
    let output_desc = format!(
        "appsrc name=src is-live=true format=time do-timestamp=true block=true caps=video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! videoconvert ! video/x-raw,format=I420 ! {} ! h265parse config-interval=1 ! rtph265pay pt=96 config-interval=1 mtu=1200 ! udpsink host={} port={} sync=false async=false",
        cfg.width, cfg.height, output_fps, enc, cfg.receiver_ip, cfg.port
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

    let appsink = match input_pipeline
        .by_name("sink")
        .and_then(|e| e.downcast::<AppSink>().ok())
    {
        Some(v) => v,
        None => {
            eprintln!("FAIL: could not find appsink in input pipeline");
            return ExitCode::from(1);
        }
    };
    let appsrc = match output_pipeline
        .by_name("src")
        .and_then(|e| e.downcast::<AppSrc>().ok())
    {
        Some(v) => v,
        None => {
            eprintln!("FAIL: could not find appsrc in output pipeline");
            return ExitCode::from(1);
        }
    };

    let cosmic_cursor = start_cosmic_cursor_tracker().ok();
    let mouse_deltas = start_mouse_delta_tracker().ok();
    let saw_cosmic_cursor = Arc::new(AtomicBool::new(false));

    let follow_state = Arc::new(Mutex::new(FollowState {
        center_x: cfg.x as f64 + cfg.width as f64 / 2.0,
        center_y: cfg.y as f64 + cfg.height as f64 / 2.0,
        cursor_x: cfg.x as f64 + cfg.width as f64 / 2.0,
        cursor_y: cfg.y as f64 + cfg.height as f64 / 2.0,
        target_x: cfg.x as f64 + cfg.width as f64 / 2.0,
        target_y: cfg.y as f64 + cfg.height as f64 / 2.0,
        follow_active: false,
        next_sample_at: Instant::now(),
        last_frame_at: Instant::now(),
    }));
    let out_idx = Arc::new(Mutex::new(0u64));
    let in_idx = Arc::new(Mutex::new(0u64));

    let follow_state_cb = Arc::clone(&follow_state);
    let out_idx_cb = Arc::clone(&out_idx);
    let in_idx_cb = Arc::clone(&in_idx);
    let appsrc_cb = appsrc.clone();
    let saw_cosmic_cursor_cb = Arc::clone(&saw_cosmic_cursor);
    let cfg_follow = cfg.follow_mouse;
    let cfg_width = cfg.width;
    let cfg_height = cfg.height;
    let cfg_x = cfg.x;
    let cfg_y = cfg.y;
    let cfg_output_fps = output_fps;
    let cfg_frame_skip = cfg.frame_skip;
    let cfg_sample_interval = cfg.sample_interval_secs;
    let cfg_smoothing = cfg.smoothing;

    appsink.set_callbacks(
        AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                let s = caps.structure(0).ok_or(gst::FlowError::Error)?;
                let src_w = s.get::<i32>("width").map_err(|_| gst::FlowError::Error)? as usize;
                let src_h = s.get::<i32>("height").map_err(|_| gst::FlowError::Error)? as usize;
                let out_w = cfg_width as usize;
                let out_h = cfg_height as usize;
                if src_w < out_w || src_h < out_h {
                    return Err(gst::FlowError::Error);
                }

                let emit_this = {
                    let mut c = in_idx_cb.lock().map_err(|_| gst::FlowError::Error)?;
                    let idx = *c;
                    *c += 1;
                    idx % u64::from(cfg_frame_skip.saturating_add(1)) == 0
                };
                if !emit_this {
                    return Ok(gst::FlowSuccess::Ok);
                }

                let now = Instant::now();
                let (crop_x, crop_y) = {
                    let mut st = follow_state_cb.lock().map_err(|_| gst::FlowError::Error)?;
                    let prev_cursor_x = st.cursor_x;
                    let prev_cursor_y = st.cursor_y;

                    if cfg_follow {
                        let mut used_cosmic = false;
                        if let Some(cosmic_xy) = &cosmic_cursor {
                            if let Ok(guard) = cosmic_xy.lock() {
                                if let Some((mx, my)) = *guard {
                                    st.cursor_x = mx;
                                    st.cursor_y = my;
                                    saw_cosmic_cursor_cb.store(true, Ordering::Relaxed);
                                    used_cosmic = true;
                                }
                            }
                        }
                        if !used_cosmic {
                            if let Some(deltas) = &mouse_deltas {
                                let mut d = deltas.lock().map_err(|_| gst::FlowError::Error)?;
                                st.cursor_x += d.0;
                                st.cursor_y += d.1;
                                d.0 = 0.0;
                                d.1 = 0.0;
                            }
                        }
                    }

                    let max_cursor_x = (src_w.saturating_sub(1)) as f64;
                    let max_cursor_y = (src_h.saturating_sub(1)) as f64;
                    st.cursor_x = st.cursor_x.clamp(0.0, max_cursor_x);
                    st.cursor_y = st.cursor_y.clamp(0.0, max_cursor_y);

                    if cfg_follow {
                        let cursor_moved = (st.cursor_x - prev_cursor_x).abs() > 0.001
                            || (st.cursor_y - prev_cursor_y).abs() > 0.001;
                        let left = (st.center_x - cfg_width as f64 / 2.0)
                            .clamp(0.0, (src_w - out_w) as f64);
                        let top = (st.center_y - cfg_height as f64 / 2.0)
                            .clamp(0.0, (src_h - out_h) as f64);
                        let right = left + cfg_width as f64;
                        let bottom = top + cfg_height as f64;
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
                            st.target_x = st.cursor_x;
                            st.target_y = st.cursor_y;
                        }
                        if now >= st.next_sample_at {
                            st.next_sample_at = now + Duration::from_secs_f64(cfg_sample_interval);
                        }
                    } else {
                        st.center_x = cfg_x as f64 + cfg_width as f64 / 2.0;
                        st.center_y = cfg_y as f64 + cfg_height as f64 / 2.0;
                        st.target_x = st.center_x;
                        st.target_y = st.center_y;
                    }

                    let dt = (now - st.last_frame_at).as_secs_f64().max(0.000_001);
                    st.last_frame_at = now;
                    let alpha = 1.0 - (-cfg_smoothing * dt).exp();
                    st.center_x += (st.target_x - st.center_x) * alpha;
                    st.center_y += (st.target_y - st.center_y) * alpha;
                    let max_x = (src_w - out_w) as f64;
                    let max_y = (src_h - out_h) as f64;
                    let cx = (st.center_x - cfg_width as f64 / 2.0).clamp(0.0, max_x).round() as usize;
                    let cy = (st.center_y - cfg_height as f64 / 2.0).clamp(0.0, max_y).round() as usize;
                    (cx, cy)
                };

                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let src = map.as_slice();
                let src_stride = src_w * 4;
                let mut out_data = vec![0u8; out_w * out_h * 4];
                for row in 0..out_h {
                    let src_off = (crop_y + row) * src_stride + crop_x * 4;
                    let dst_off = row * out_w * 4;
                    out_data[dst_off..dst_off + out_w * 4]
                        .copy_from_slice(&src[src_off..src_off + out_w * 4]);
                }

                let mut out_buf = gst::Buffer::from_mut_slice(out_data);
                {
                    let idx = {
                        let mut c = out_idx_cb.lock().map_err(|_| gst::FlowError::Error)?;
                        let v = *c;
                        *c += 1;
                        v
                    };
                    let dur =
                        gst::ClockTime::from_nseconds(1_000_000_000u64 / cfg_output_fps as u64);
                    let pts = gst::ClockTime::from_nseconds(
                        (1_000_000_000u64 * idx) / cfg_output_fps as u64,
                    );
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

    let in_bus = match input_pipeline.bus() {
        Some(v) => v,
        None => {
            let _ = input_pipeline.set_state(gst::State::Null);
            let _ = output_pipeline.set_state(gst::State::Null);
            eprintln!("FAIL: could not get input bus");
            return ExitCode::from(1);
        }
    };
    let out_bus = match output_pipeline.bus() {
        Some(v) => v,
        None => {
            let _ = input_pipeline.set_state(gst::State::Null);
            let _ = output_pipeline.set_state(gst::State::Null);
            eprintln!("FAIL: could not get output bus");
            return ExitCode::from(1);
        }
    };

    let mut done = false;
    let deadline = Instant::now() + Duration::from_secs(8 * 60 * 60);
    while Instant::now() < deadline {
        if let Some(msg) = in_bus.timed_pop(gst::ClockTime::from_mseconds(50)) {
            match msg.view() {
                gst::MessageView::Error(e) => {
                    eprintln!(
                        "FAIL: input pipeline error from {}: {}",
                        e.src().map(|s| s.path_string()).unwrap_or_else(|| "<unknown>".into()),
                        e.error()
                    );
                    done = true;
                }
                gst::MessageView::Eos(..) => done = true,
                _ => {}
            }
        }
        if let Some(msg) = out_bus.timed_pop(gst::ClockTime::from_mseconds(0)) {
            match msg.view() {
                gst::MessageView::Error(e) => {
                    eprintln!(
                        "FAIL: output pipeline error from {}: {}",
                        e.src().map(|s| s.path_string()).unwrap_or_else(|| "<unknown>".into()),
                        e.error()
                    );
                    done = true;
                }
                gst::MessageView::Eos(..) => done = true,
                _ => {}
            }
        }
        if done {
            break;
        }
    }

    let _ = input_pipeline.set_state(gst::State::Null);
    let _ = output_pipeline.set_state(gst::State::Null);
    if done {
        ExitCode::SUCCESS
    } else {
        eprintln!("FAIL: sender timed out");
        ExitCode::from(1)
    }
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
        let session =
            tokio::time::timeout(Duration::from_secs(PORTAL_TIMEOUT_SECS), portal.create_session())
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
        let request =
            tokio::time::timeout(Duration::from_secs(PORTAL_TIMEOUT_SECS), portal.start(&session, None))
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
    _cursor_session: Option<CaptureCursorSession>,
    cursor_xy: Arc<Mutex<Option<(f64, f64)>>>,
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
    fn new_output(&mut self, _: &WlConnection, _: &WlQueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(
        &mut self,
        _: &WlConnection,
        _: &WlQueueHandle<Self>,
        _: wl_output::WlOutput,
    ) {
    }
    fn output_destroyed(
        &mut self,
        _: &WlConnection,
        _: &WlQueueHandle<Self>,
        _: wl_output::WlOutput,
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
        _: &WlConnection,
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
        _: &WlConnection,
        _: &WlQueueHandle<Self>,
        _: wl_seat::WlSeat,
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
        _: &WlConnection,
        _: &WlQueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        _: &[PointerEvent],
    ) {
    }
}

impl ScreencopyHandler for CosmicCursorApp {
    fn screencopy_state(&mut self) -> &mut ScreencopyState {
        &mut self.screencopy_state
    }
    fn init_done(
        &mut self,
        _: &WlConnection,
        _: &WlQueueHandle<Self>,
        _: &CaptureSession,
        _: &Formats,
    ) {
    }
    fn stopped(&mut self, _: &WlConnection, _: &WlQueueHandle<Self>, _: &CaptureSession) {}
    fn ready(&mut self, _: &WlConnection, _: &WlQueueHandle<Self>, _: &CaptureFrame, _: Frame) {}
    fn failed(
        &mut self,
        _: &WlConnection,
        _: &WlQueueHandle<Self>,
        _: &CaptureFrame,
        _: WEnum<FailureReason>,
    ) {
    }
    fn cursor_position(
        &mut self,
        _: &WlConnection,
        _: &WlQueueHandle<Self>,
        _: &CaptureCursorSession,
        x: i32,
        y: i32,
    ) {
        if let Ok(mut cursor_xy) = self.cursor_xy.lock() {
            *cursor_xy = Some((x as f64, y as f64));
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
        _cursor_session: None,
        cursor_xy,
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

    let session = app
        .screencopy_state
        .capturer()
        .create_cursor_session(
            &CaptureSource::Output(output),
            &pointer,
            &qh,
            CursorSessionData::default(),
        )
        .map_err(|e| format!("create_cursor_session failed: {e}"))?;
    app._cursor_session = Some(session);
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
    let entries = std::fs::read_dir("/dev/input")
        .map_err(|e| format!("failed to scan /dev/input: {e}"))?;
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
            if let Ok(events) = dev.fetch_events() {
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
        }
        thread::sleep(Duration::from_millis(2));
    });
    Ok(deltas)
}

fn print_help() {
    println!("vp-sndr: HEVC RTP sender");
    println!();
    println!("Usage:");
    println!("  vp-sndr send --receiver-ip IP [--port N] [--x N] [--y N] [--width N] [--height N] [--fps N] [--frame-skip N] [--follow-mouse] [--sample-interval S] [--smoothing K] [--encoder x265enc|vaapih265enc|v4l2h265enc] [--bitrate-kbps N]");
}
