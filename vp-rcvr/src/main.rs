use ksni::menu::{MenuItem, StandardItem};
use ksni::{Tray, TrayService};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReceiverConfig {
    codec: String,
    bind_ip: String,
    port: u16,
    payload: u8,
    clock_rate: u32,
    latency_ms: u32,
    no_preview: bool,
    v4l2_device: Option<String>,
    v4l2_width: Option<u32>,
    v4l2_height: Option<u32>,
    v4l2_fps: Option<u32>,
}

impl Default for ReceiverConfig {
    fn default() -> Self {
        Self {
            codec: "h265".to_string(),
            bind_ip: "0.0.0.0".to_string(),
            port: 5000,
            payload: 96,
            clock_rate: 90_000,
            latency_ms: 25,
            no_preview: false,
            v4l2_device: None,
            v4l2_width: None,
            v4l2_height: None,
            v4l2_fps: None,
        }
    }
}

fn config_path() -> Result<PathBuf, String> {
    let mut dir = dirs::config_dir().ok_or_else(|| "could not resolve config directory".to_string())?;
    dir.push("vp-link");
    dir.push("vp-rcvr.toml");
    Ok(dir)
}

fn load_config() -> ReceiverConfig {
    let path = match config_path() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("WARN: {err}");
            return ReceiverConfig::default();
        }
    };
    let data = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return ReceiverConfig::default(),
    };
    match toml::from_str::<ReceiverConfig>(&data) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("WARN: could not parse {}: {err}", path.display());
            ReceiverConfig::default()
        }
    }
}

fn save_config(cfg: &ReceiverConfig) -> Result<(), String> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    let data = toml::to_string_pretty(cfg).map_err(|e| format!("serialize config: {e}"))?;
    fs::write(&path, data).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

fn cfg_from_receive(
    codec: &str,
    bind_ip: &str,
    port: u16,
    payload: u8,
    clock_rate: u32,
    latency_ms: u32,
    no_preview: bool,
    v4l2_device: Option<&str>,
    v4l2_width: Option<u32>,
    v4l2_height: Option<u32>,
    v4l2_fps: Option<u32>,
) -> ReceiverConfig {
    ReceiverConfig {
        codec: codec.to_string(),
        bind_ip: bind_ip.to_string(),
        port,
        payload,
        clock_rate,
        latency_ms,
        no_preview,
        v4l2_device: v4l2_device.map(|v| v.to_string()),
        v4l2_width,
        v4l2_height,
        v4l2_fps,
    }
}

#[derive(Clone, Default)]
struct ReceiverTray;

impl Tray for ReceiverTray {
    fn id(&self) -> String {
        "vp-rcvr".to_string()
    }

    fn title(&self) -> String {
        "vp-rcvr".to_string()
    }

    fn icon_name(&self) -> String {
        "video-display".to_string()
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let running = service_is_active("vp-rcvr.service");
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
                label: "Stop Receiver".to_string(),
                activate: Box::new(move |_| tray_stop()),
                ..Default::default()
            }));
        } else {
            items.push(MenuItem::Standard(StandardItem {
                label: "Start Receiver".to_string(),
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
    let tray = ReceiverTray;
    let service = TrayService::new(tray);
    let _handle = service.spawn();
    loop {
        std::thread::park();
    }
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
    service_action("vp-rcvr.service", "start");
}

fn tray_stop() {
    service_action("vp-rcvr.service", "stop");
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
            run_receive(
                &cfg.codec,
                &cfg.bind_ip,
                cfg.port,
                cfg.payload,
                cfg.clock_rate,
                cfg.latency_ms,
                !cfg.no_preview,
                cfg.v4l2_device.as_deref(),
                cfg.v4l2_width,
                cfg.v4l2_height,
                cfg.v4l2_fps,
            )
        }
        Ok(Cli::Receive {
            codec,
            bind_ip,
            port,
            payload,
            clock_rate,
            latency_ms,
            no_preview,
            v4l2_device,
            v4l2_width,
            v4l2_height,
            v4l2_fps,
        }) => {
            if let Err(err) = save_config(&cfg_from_receive(
                &codec,
                &bind_ip,
                port,
                payload,
                clock_rate,
                latency_ms,
                no_preview,
                v4l2_device.as_deref(),
                v4l2_width,
                v4l2_height,
                v4l2_fps,
            )) {
                eprintln!("WARN: {err}");
            }
            run_receive(
                &codec,
                &bind_ip,
                port,
                payload,
                clock_rate,
                latency_ms,
                !no_preview,
                v4l2_device.as_deref(),
                v4l2_width,
                v4l2_height,
                v4l2_fps,
            )
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
    Receive {
        codec: String,
        bind_ip: String,
        port: u16,
        payload: u8,
        clock_rate: u32,
        latency_ms: u32,
        no_preview: bool,
        v4l2_device: Option<String>,
        v4l2_width: Option<u32>,
        v4l2_height: Option<u32>,
        v4l2_fps: Option<u32>,
    },
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
        "receive" => {
            let mut bind_ip = String::from("0.0.0.0");
            let mut codec = String::from("h265");
            let mut port = 5000u16;
            let mut payload = 96u8;
            let mut clock_rate = 90_000u32;
            let mut latency_ms = 25u32;
            let mut no_preview = false;
            let mut v4l2_device: Option<String> = None;
            let mut v4l2_width: Option<u32> = None;
            let mut v4l2_height: Option<u32> = None;
            let mut v4l2_fps: Option<u32> = None;

            let mut i = 2usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--bind-ip" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --bind-ip".to_string())?;
                        bind_ip = next.clone();
                        i += 2;
                    }
                    "--codec" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --codec".to_string())?;
                        let next_lc = next.to_ascii_lowercase();
                        if next_lc != "h264" && next_lc != "h265" {
                            return Err(format!("invalid --codec value: {next} (expected h264 or h265)"));
                        }
                        codec = next_lc;
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
                    "--payload" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --payload".to_string())?;
                        payload = next
                            .parse::<u8>()
                            .map_err(|_| format!("invalid --payload value: {next}"))?;
                        i += 2;
                    }
                    "--clock-rate" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --clock-rate".to_string())?;
                        clock_rate = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --clock-rate value: {next}"))?;
                        i += 2;
                    }
                    "--latency-ms" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --latency-ms".to_string())?;
                        latency_ms = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --latency-ms value: {next}"))?;
                        i += 2;
                    }
                    "--no-preview" => {
                        no_preview = true;
                        i += 1;
                    }
                    "--v4l2-device" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --v4l2-device".to_string())?;
                        v4l2_device = Some(next.clone());
                        i += 2;
                    }
                    "--v4l2-width" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --v4l2-width".to_string())?;
                        let val = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --v4l2-width value: {next}"))?;
                        if val == 0 {
                            return Err("--v4l2-width must be > 0".to_string());
                        }
                        v4l2_width = Some(val);
                        i += 2;
                    }
                    "--v4l2-height" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --v4l2-height".to_string())?;
                        let val = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --v4l2-height value: {next}"))?;
                        if val == 0 {
                            return Err("--v4l2-height must be > 0".to_string());
                        }
                        v4l2_height = Some(val);
                        i += 2;
                    }
                    "--v4l2-fps" => {
                        let next = args
                            .get(i + 1)
                            .ok_or_else(|| "missing value after --v4l2-fps".to_string())?;
                        let val = next
                            .parse::<u32>()
                            .map_err(|_| format!("invalid --v4l2-fps value: {next}"))?;
                        if val == 0 {
                            return Err("--v4l2-fps must be > 0".to_string());
                        }
                        v4l2_fps = Some(val);
                        i += 2;
                    }
                    other => return Err(format!("unknown argument: {other}")),
                }
            }

            if no_preview && v4l2_device.is_none() {
                return Err(
                    "nothing to do: provide preview or --v4l2-device when using --no-preview"
                        .to_string(),
                );
            }

            Ok(Cli::Receive {
                codec,
                bind_ip,
                port,
                payload,
                clock_rate,
                latency_ms,
                no_preview,
                v4l2_device,
                v4l2_width,
                v4l2_height,
                v4l2_fps,
            })
        }
        other => Err(format!("unknown command: {other}")),
    }
}

fn run_receive(
    codec: &str,
    bind_ip: &str,
    port: u16,
    payload: u8,
    clock_rate: u32,
    latency_ms: u32,
    preview: bool,
    v4l2_device: Option<&str>,
    v4l2_width: Option<u32>,
    v4l2_height: Option<u32>,
    v4l2_fps: Option<u32>,
) -> ExitCode {
    let (encoding_name, depay_parse, decode_chain) = match codec {
        "h264" => ("H264", "rtph264depay ! h264parse", "decodebin"),
        "h265" => (
            "H265",
            "rtph265depay ! h265parse",
            "avdec_h265 output-corrupt=false discard-corrupted-frames=true",
        ),
        other => {
            eprintln!("FAIL: unsupported codec '{other}'");
            return ExitCode::from(2);
        }
    };
    let caps = format!(
        "application/x-rtp,media=video,encoding-name={encoding_name},payload={payload},clock-rate={clock_rate}"
    );

    let mut pipeline = format!(
        "udpsrc address={bind_ip} port={port} caps=\"{caps}\" ! \
         queue ! rtpjitterbuffer latency={latency_ms} drop-on-latency=true ! \
         {depay_parse} ! {decode_chain} ! tee name=t"
    );

    if preview {
        pipeline.push_str(
            " t. ! queue ! videoconvert ! fpsdisplaysink text-overlay=false video-sink=autovideosink sync=false",
        );
    }

    if let Some(device) = v4l2_device {
        let mut v4l2_caps = String::from("video/x-raw,format=I420");
        if let Some(w) = v4l2_width {
            v4l2_caps.push_str(&format!(",width={w}"));
        }
        if let Some(h) = v4l2_height {
            v4l2_caps.push_str(&format!(",height={h}"));
        }
        if let Some(fps) = v4l2_fps {
            v4l2_caps.push_str(&format!(",framerate={fps}/1"));
        }
        pipeline.push_str(&format!(
            " t. ! queue ! videoconvert ! {} ! v4l2sink device={} sync=false",
            v4l2_caps, device
        ));
    }

    println!("Starting {} receiver on {}:{}...", encoding_name, bind_ip, port);
    println!("Pipeline: {}", pipeline);

    let cmd = format!("gst-launch-1.0 -e -v {pipeline}");
    let status = Command::new("bash")
        .args(["-lc", &cmd])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    match status {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(s) => {
            eprintln!(
                "FAIL: gst-launch-1.0 exited with code {}",
                s.code().unwrap_or(-1)
            );
            ExitCode::from(1)
        }
        Err(err) => {
            eprintln!("FAIL: could not start gst-launch-1.0: {err}");
            ExitCode::from(1)
        }
    }
}

fn print_help() {
    println!("vp-rcvr: HEVC viewport receiver");
    println!();
    println!("Usage:");
    println!("  vp-rcvr receive [--codec h264|h265] [--bind-ip IP] [--port N] [--payload N] [--clock-rate N] [--latency-ms N] [--no-preview] [--v4l2-device /dev/videoN] [--v4l2-width N] [--v4l2-height N] [--v4l2-fps N]");
    println!("  vp-rcvr tray");
    println!("  vp-rcvr config");
    println!("  vp-rcvr run-saved");
    println!();
    println!("Examples:");
    println!("  vp-rcvr receive --port 5000");
    println!("  vp-rcvr receive --port 5000 --v4l2-device /dev/video10");
    println!("  vp-rcvr receive --port 5000 --no-preview --v4l2-device /dev/video10");
    println!("  vp-rcvr receive --codec h264 --port 5000 --no-preview --v4l2-device /dev/video10 --v4l2-width 1280 --v4l2-height 720 --v4l2-fps 60");
    println!("  vp-rcvr tray");
    println!("  vp-rcvr config");
    println!("  vp-rcvr run-saved");
}
