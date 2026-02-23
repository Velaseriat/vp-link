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
use gstreamer_video as gst_video;
use ksni::menu::{MenuItem, StandardItem};
use ksni::{Icon, Tray, TrayService};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::env;
use std::fs;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const PORTAL_TIMEOUT_SECS: u64 = 15;
const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const DEFAULT_QUEUE_BUFFERS: u32 = 8;
const DEFAULT_MOUSE_SMOOTHING: f64 = 8.0;
const DEFAULT_CURSOR_CHANGE_EPSILON_PX: f64 = 0.25;
const DEFAULT_SETTLE_EPSILON_PX: f64 = 0.75;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SenderConfig {
    receiver_ip: String,
    port: u16,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    fps: u32,
    follow_mouse: bool,
    smoothing: f64,
    deadzone: f64,
    encoder: String,
    bitrate_kbps: u32,
}

impl Default for SenderConfig {
    fn default() -> Self {
        Self {
            receiver_ip: "127.0.0.1".to_string(),
            port: 5000,
            x: 0,
            y: 0,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            fps: 60,
            follow_mouse: false,
            smoothing: DEFAULT_MOUSE_SMOOTHING,
            deadzone: 0.0,
            encoder: "x265enc".to_string(),
            bitrate_kbps: 8000,
        }
    }
}

fn config_path() -> Result<PathBuf, String> {
    let mut dir = dirs::config_dir().ok_or_else(|| "could not resolve config directory".to_string())?;
    dir.push("vp-link");
    dir.push("vp-sndr.toml");
    Ok(dir)
}

fn load_config() -> SenderConfig {
    let path = match config_path() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("WARN: {err}");
            return SenderConfig::default();
        }
    };
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return SenderConfig::default(),
    };
    match toml::from_str::<SenderConfig>(&data) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("WARN: could not parse {}: {err}", path.display());
            SenderConfig::default()
        }
    }
}

fn save_config(cfg: &SenderConfig) -> Result<(), String> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    let data = toml::to_string_pretty(cfg).map_err(|e| format!("serialize config: {e}"))?;
    fs::write(&path, data).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

fn cfg_from_send(cfg: &SendCfg) -> SenderConfig {
    SenderConfig {
        receiver_ip: cfg.receiver_ip.clone(),
        port: cfg.port,
        x: cfg.x,
        y: cfg.y,
        width: cfg.width,
        height: cfg.height,
        fps: cfg.fps,
        follow_mouse: cfg.follow_mouse,
        smoothing: cfg.smoothing,
        deadzone: cfg.deadzone,
        encoder: cfg.encoder.clone(),
        bitrate_kbps: cfg.bitrate_kbps,
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match parse_cli(&args) {
        Ok(Cli::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(Cli::ConfigPath) => {
            match config_path() {
                Ok(path) => println!("{}", path.display()),
                Err(err) => eprintln!("error: {err}"),
            }
            ExitCode::SUCCESS
        }
        Ok(Cli::Tray) => run_tray(),
        Ok(Cli::RunSaved) => {
            let cfg = load_config();
            run_send(SendCfg {
                receiver_ip: cfg.receiver_ip,
                port: cfg.port,
                x: cfg.x,
                y: cfg.y,
                width: cfg.width,
                height: cfg.height,
                fps: cfg.fps,
                follow_mouse: cfg.follow_mouse,
                smoothing: cfg.smoothing,
                deadzone: cfg.deadzone,
                encoder: cfg.encoder,
                bitrate_kbps: cfg.bitrate_kbps,
            })
        }
        Ok(Cli::Send {
            receiver_ip,
            port,
            x,
            y,
            width,
            height,
            fps,
            follow_mouse,
            smoothing,
            deadzone,
            encoder,
            bitrate_kbps,
        }) => {
            let send_cfg = SendCfg {
                receiver_ip,
                port,
                x,
                y,
                width,
                height,
                fps,
                follow_mouse,
                smoothing,
                deadzone,
                encoder,
                bitrate_kbps,
            };
            if let Err(err) = save_config(&cfg_from_send(&send_cfg)) {
                eprintln!("WARN: {err}");
            }
            run_send(send_cfg)
        }
        Err(err) => {
            eprintln!("error: {err}");
            print_help();
            ExitCode::from(2)
        }
    }
}

enum Cli {
    Help,
    Tray,
    ConfigPath,
    RunSaved,
    Send {
        receiver_ip: String,
        port: u16,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        fps: u32,
        follow_mouse: bool,
        smoothing: f64,
        deadzone: f64,
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
    follow_mouse: bool,
    smoothing: f64,
    deadzone: f64,
    encoder: String,
    bitrate_kbps: u32,
}

#[derive(Clone, Default)]
struct SenderTray;

impl Tray for SenderTray {
    fn id(&self) -> String {
        "vp-sndr".to_string()
    }

    fn title(&self) -> String {
        "vp-sndr".to_string()
    }

    fn icon_name(&self) -> String {
        "video-display".to_string()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        // Fallback icon for trays that do not resolve icon_name from theme.
        let width = 16i32;
        let height = 16i32;
        let mut data = vec![0u8; (width * height * 4) as usize];
        for px in data.chunks_exact_mut(4) {
            // ARGB32 network byte order: A, R, G, B.
            px[0] = 0xFF;
            px[1] = 0xE5;
            px[2] = 0x39;
            px[3] = 0x35;
        }
        vec![Icon {
            width,
            height,
            data,
        }]
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let running = service_is_active("vp-sndr.service");
        let status_label = if running {
            "Service: running"
        } else {
            "Service: stopped"
        };
        let mut items = vec![MenuItem::Standard(StandardItem {
            label: status_label.to_string(),
            enabled: false,
            ..Default::default()
        })];
        if running {
            items.push(MenuItem::Standard(StandardItem {
                label: "Stop Sender".to_string(),
                activate: Box::new(move |_| tray_stop()),
                ..Default::default()
            }));
        } else {
            items.push(MenuItem::Standard(StandardItem {
                label: "Start Sender".to_string(),
                activate: Box::new(move |_| tray_start()),
                ..Default::default()
            }));
        }
        items.push(MenuItem::Standard(StandardItem {
            label: "Open Config".to_string(),
            activate: Box::new(move |_| tray_open_config()),
            ..Default::default()
        }));
        items.push(MenuItem::Standard(StandardItem {
            label: "Quit".to_string(),
            activate: Box::new(move |_| std::process::exit(0)),
            ..Default::default()
        }));
        items
    }
}

fn run_tray() -> ExitCode {
    if let Err(err) = ensure_session_bus_available() {
        eprintln!("ERROR: {err}");
        eprintln!("Run `vp-sndr tray` from an active desktop session (not plain SSH).");
        return ExitCode::from(1);
    }

    let tray = SenderTray;
    let service = TrayService::new(tray);
    let _handle = service.spawn();
    loop {
        std::thread::park();
    }
}

fn ensure_session_bus_available() -> Result<(), String> {
    let addr = env::var("DBUS_SESSION_BUS_ADDRESS")
        .map_err(|_| "DBUS_SESSION_BUS_ADDRESS is not set".to_string())?;
    if addr.trim().is_empty() {
        return Err("DBUS_SESSION_BUS_ADDRESS is empty".to_string());
    }

    let path = if let Some(rest) = addr.strip_prefix("unix:path=") {
        rest.split(',').next().unwrap_or("")
    } else if addr.starts_with('/') {
        addr.as_str()
    } else {
        return Err(format!(
            "unsupported DBus address format: {addr} (expected unix:path=...)"
        ));
    };

    if path.is_empty() {
        return Err(format!("invalid DBus address: {addr}"));
    }

    UnixStream::connect(path)
        .map(|_| ())
        .map_err(|e| format!("cannot connect to session bus at {path}: {e}"))
}

fn service_is_active(service: &str) -> bool {
    match Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", service])
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

fn service_action(service: &str, action: &str) {
    let status = Command::new("systemctl")
        .args(["--user", action, service])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    if let Err(err) = status {
        eprintln!("WARN: systemctl --user {action} {service} failed: {err}");
    }
}

fn tray_start() {
    service_action("vp-sndr.service", "start");
}

fn tray_stop() {
    service_action("vp-sndr.service", "stop");
}

fn tray_open_config() {
    let cfg = load_config();
    let _ = save_config(&cfg);
    let path = match config_path() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("WARN: {err}");
            return;
        }
    };
    let _ = Command::new("xdg-open")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn parse_cli(args: &[String]) -> Result<Cli, String> {
    if args.len() <= 1 {
        return Ok(Cli::Help);
    }
    match args[1].as_str() {
        "-h" | "--help" | "help" => Ok(Cli::Help),
        "tray" => Ok(Cli::Tray),
        "config" => Ok(Cli::ConfigPath),
        "run-saved" => Ok(Cli::RunSaved),
        "send" => {
            let mut receiver_ip: Option<String> = None;
            let mut port = 5000u16;
            let mut x = 0u32;
            let mut y = 0u32;
            let mut width = DEFAULT_WIDTH;
            let mut height = DEFAULT_HEIGHT;
            let mut fps = 60u32;
            let mut follow_mouse = false;
            let mut smoothing = DEFAULT_MOUSE_SMOOTHING;
            let mut deadzone = 0.0f64;
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
                    "--follow-mouse" => {
                        follow_mouse = true;
                        i += 1;
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
                    "--deadzone" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --deadzone".to_string())?;
                        deadzone = next
                            .parse::<f64>()
                            .map_err(|_| format!("invalid --deadzone value: {next}"))?;
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
            if smoothing <= 0.0 {
                return Err("--smoothing must be > 0".to_string());
            }
            if !(0.0..=100.0).contains(&deadzone) {
                return Err("--deadzone must be between 0 and 100".to_string());
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
                follow_mouse,
                smoothing,
                deadzone,
                encoder,
                bitrate_kbps,
            })
        }
        other => Err(format!("unknown command: {other}")),
    }
}

fn run_send(cfg: SendCfg) -> ExitCode {
    let output_fps = cfg.fps.max(1);
    println!(
        "Sending to {}:{} capture_fps={} crop={}x{} at x={}, y={}",
        cfg.receiver_ip,
        cfg.port,
        cfg.fps,
        cfg.width,
        cfg.height,
        cfg.x,
        cfg.y
    );
    if cfg.follow_mouse {
        println!("Mouse follow enabled (smoothing={}).", cfg.smoothing);
        if cfg.deadzone > 0.0 {
            println!("Deadzone enabled ({}% x {}%).", cfg.deadzone, cfg.deadzone);
        }
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
    is_lerping: bool,
    last_frame_at: Instant,
}

fn encoder_stage(encoder: &str, fps: u32, bitrate_kbps: u32) -> Result<String, String> {
    match encoder {
        "x264enc" => Ok(format!(
            "x264enc tune=zerolatency speed-preset=ultrafast key-int-max={} bitrate={}",
            fps.max(1),
            bitrate_kbps
        )),
        "nvh264enc" => Ok(format!(
            "nvh264enc bitrate={} gop-size={}",
            bitrate_kbps,
            fps.max(1)
        )),
        "x265enc" => {
            let gop = (fps.max(1) * 2).max(30);
            Ok(format!(
                "x265enc speed-preset=veryfast key-int-max={} bitrate={} option-string=\"repeat-headers=1:aud=1:scenecut=0\"",
                gop,
                bitrate_kbps
            ))
        }
        "nvh265enc" => Ok(format!(
            "nvh265enc bitrate={} gop-size={}",
            bitrate_kbps,
            fps.max(1)
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

fn rtp_video_stage(encoder: &str) -> Result<&'static str, String> {
    match encoder {
        "x264enc" | "nvh264enc" => {
            Ok("h264parse config-interval=1 ! rtph264pay pt=96 config-interval=1 mtu=1200")
        }
        "x265enc" | "nvh265enc" | "vaapih265enc" | "v4l2h265enc" => {
            Ok("h265parse config-interval=1 ! rtph265pay pt=96 config-interval=1 mtu=1200")
        }
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
    let rtp_stage = match rtp_video_stage(&cfg.encoder) {
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
        "appsrc name=src is-live=true format=time do-timestamp=true block=true caps=video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! queue max-size-buffers={} max-size-bytes=0 max-size-time=0 ! videoconvert ! video/x-raw,format=I420 ! queue max-size-buffers={} max-size-bytes=0 max-size-time=0 ! {} ! queue max-size-buffers={} max-size-bytes=0 max-size-time=0 ! {} ! queue max-size-buffers={} max-size-bytes=0 max-size-time=0 ! udpsink host={} port={} sync=false async=false",
        cfg.width, cfg.height, output_fps, DEFAULT_QUEUE_BUFFERS, DEFAULT_QUEUE_BUFFERS, enc, DEFAULT_QUEUE_BUFFERS, rtp_stage, DEFAULT_QUEUE_BUFFERS, cfg.receiver_ip, cfg.port
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
        is_lerping: false,
        last_frame_at: Instant::now(),
    }));
    let out_idx = Arc::new(Mutex::new(0u64));

    let follow_state_cb = Arc::clone(&follow_state);
    let out_idx_cb = Arc::clone(&out_idx);
    let appsrc_cb = appsrc.clone();
    let saw_cosmic_cursor_cb = Arc::clone(&saw_cosmic_cursor);
    let cfg_follow = cfg.follow_mouse;
    let cfg_width = cfg.width;
    let cfg_height = cfg.height;
    let cfg_x = cfg.x;
    let cfg_y = cfg.y;
    let cfg_output_fps = output_fps;
    let cfg_smoothing = cfg.smoothing;
    let cfg_deadzone = cfg.deadzone;

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

                let now = Instant::now();
                let (crop_x, crop_y) = {
                    let mut st = follow_state_cb.lock().map_err(|_| gst::FlowError::Error)?;
                    let prev_cursor_x = st.cursor_x;
                    let prev_cursor_y = st.cursor_y;

                    if cfg_follow {
                        let mut used_stream_meta = false;
                        if let Some((mx, my)) = extract_cursor_from_sample(&sample, src_w as u32, src_h as u32) {
                            st.cursor_x = mx;
                            st.cursor_y = my;
                            used_stream_meta = true;
                        }

                        let mut used_cosmic = false;
                        if !used_stream_meta {
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
                        }

                        if !used_stream_meta && !used_cosmic {
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
                        let cursor_changed = (st.cursor_x - prev_cursor_x).abs() > DEFAULT_CURSOR_CHANGE_EPSILON_PX
                            || (st.cursor_y - prev_cursor_y).abs() > DEFAULT_CURSOR_CHANGE_EPSILON_PX;
                        if cursor_changed {
                            if cfg_deadzone > 0.0 {
                                let dz_half_w = (cfg_width as f64) * (cfg_deadzone / 100.0) / 2.0;
                                let dz_half_h = (cfg_height as f64) * (cfg_deadzone / 100.0) / 2.0;
                                let left = st.center_x - dz_half_w;
                                let right = st.center_x + dz_half_w;
                                let top = st.center_y - dz_half_h;
                                let bottom = st.center_y + dz_half_h;

                                let target_x = if st.cursor_x < left {
                                    st.cursor_x + dz_half_w
                                } else if st.cursor_x > right {
                                    st.cursor_x - dz_half_w
                                } else {
                                    st.center_x
                                };
                                let target_y = if st.cursor_y < top {
                                    st.cursor_y + dz_half_h
                                } else if st.cursor_y > bottom {
                                    st.cursor_y - dz_half_h
                                } else {
                                    st.center_y
                                };
                                st.target_x = target_x;
                                st.target_y = target_y;
                            } else {
                                st.target_x = st.cursor_x;
                                st.target_y = st.cursor_y;
                            }
                            st.is_lerping = true;
                        }
                    } else {
                        st.center_x = cfg_x as f64 + cfg_width as f64 / 2.0;
                        st.center_y = cfg_y as f64 + cfg_height as f64 / 2.0;
                        st.target_x = st.center_x;
                        st.target_y = st.center_y;
                        st.is_lerping = false;
                    }

                    let dt = (now - st.last_frame_at).as_secs_f64().max(0.000_001);
                    st.last_frame_at = now;
                    if st.is_lerping {
                        let alpha = 1.0 - (-cfg_smoothing * dt).exp();
                        st.center_x += (st.target_x - st.center_x) * alpha;
                        st.center_y += (st.target_y - st.center_y) * alpha;
                        let dx = st.target_x - st.center_x;
                        let dy = st.target_y - st.center_y;
                        let settle2 = DEFAULT_SETTLE_EPSILON_PX * DEFAULT_SETTLE_EPSILON_PX;
                        if dx * dx + dy * dy <= settle2 {
                            st.center_x = st.target_x;
                            st.center_y = st.target_y;
                            st.is_lerping = false;
                        }
                    }
                    let max_x = (src_w - out_w) as f64;
                    let max_y = (src_h - out_h) as f64;
                    let cx = (st.center_x - cfg_width as f64 / 2.0).clamp(0.0, max_x).round() as usize;
                    let cy = (st.center_y - cfg_height as f64 / 2.0).clamp(0.0, max_y).round() as usize;
                    (cx, cy)
                };

                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let (plane0_offset, src_stride) = if let Some(meta) = buffer.meta::<gst_video::VideoMeta>() {
                    let offset = meta.offset().first().copied().unwrap_or(0);
                    let stride = meta
                        .stride()
                        .first()
                        .copied()
                        .filter(|v| *v > 0)
                        .map(|v| v as usize)
                        .unwrap_or(src_w * 4);
                    (offset, stride)
                } else {
                    (0usize, src_w * 4)
                };
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let src = map.as_slice();
                let mut out_data = vec![0u8; out_w * out_h * 4];
                for row in 0..out_h {
                    let src_off = plane0_offset + (crop_y + row) * src_stride + crop_x * 4;
                    let dst_off = row * out_w * 4;
                    let src_end = src_off + out_w * 4;
                    if src_end > src.len() {
                        return Err(gst::FlowError::Error);
                    }
                    out_data[dst_off..dst_off + out_w * 4]
                        .copy_from_slice(&src[src_off..src_end]);
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

fn print_help() {
    println!("vp-sndr: HEVC RTP sender");
    println!();
    println!("Usage:");
    println!("  vp-sndr send --receiver-ip IP [--port N] [--x N] [--y N] [--width N] [--height N] [--fps N] [--follow-mouse] [--smoothing K] [--deadzone PCT] [--encoder x264enc|nvh264enc|x265enc|nvh265enc|vaapih265enc|v4l2h265enc] [--bitrate-kbps N]");
    println!("  vp-sndr tray");
    println!("  vp-sndr config");
    println!("  vp-sndr run-saved");
    println!();
    println!("Examples:");
    println!("  vp-sndr send --receiver-ip 192.168.1.50 --port 5000 --x 200 --y 100 --width 1280 --height 720 --fps 60 --follow-mouse --smoothing 4 --deadzone 30 --encoder x265enc --bitrate-kbps 8000");
    println!("  vp-sndr tray");
    println!("  vp-sndr config");
    println!("  vp-sndr run-saved");
}
