use std::env;
use std::process::{Command, ExitCode, Stdio};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match parse_cli(&args) {
        Ok(Cli::Help) => {
            print_help();
            ExitCode::SUCCESS
        }
        Ok(Cli::Receive {
            bind_ip,
            port,
            payload,
            clock_rate,
            latency_ms,
            no_preview,
            v4l2_device,
        }) => run_receive(
            &bind_ip,
            port,
            payload,
            clock_rate,
            latency_ms,
            !no_preview,
            v4l2_device.as_deref(),
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
    Receive {
        bind_ip: String,
        port: u16,
        payload: u8,
        clock_rate: u32,
        latency_ms: u32,
        no_preview: bool,
        v4l2_device: Option<String>,
    },
}

fn parse_cli(args: &[String]) -> Result<Cli, String> {
    if args.len() <= 1 {
        return Ok(Cli::Help);
    }
    match args[1].as_str() {
        "-h" | "--help" | "help" => Ok(Cli::Help),
        "receive" => {
            let mut bind_ip = String::from("0.0.0.0");
            let mut port = 5000u16;
            let mut payload = 96u8;
            let mut clock_rate = 90_000u32;
            let mut latency_ms = 25u32;
            let mut no_preview = false;
            let mut v4l2_device: Option<String> = None;

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
                bind_ip,
                port,
                payload,
                clock_rate,
                latency_ms,
                no_preview,
                v4l2_device,
            })
        }
        other => Err(format!("unknown command: {other}")),
    }
}

fn run_receive(
    bind_ip: &str,
    port: u16,
    payload: u8,
    clock_rate: u32,
    latency_ms: u32,
    preview: bool,
    v4l2_device: Option<&str>,
) -> ExitCode {
    let caps = format!(
        "application/x-rtp,media=video,encoding-name=H265,payload={payload},clock-rate={clock_rate}"
    );

    let mut pipeline = format!(
        "udpsrc address={bind_ip} port={port} caps=\"{caps}\" ! \
         rtpjitterbuffer latency={latency_ms} drop-on-latency=true ! \
         rtph265depay ! h265parse ! tee name=t"
    );

    if preview {
        pipeline.push_str(
            " t. ! queue ! decodebin ! videoconvert ! \
             fpsdisplaysink text-overlay=false video-sink=autovideosink sync=false",
        );
    }

    if let Some(device) = v4l2_device {
        pipeline.push_str(&format!(
            " t. ! queue ! decodebin ! videoconvert ! v4l2sink device={} sync=false",
            device
        ));
    }

    println!("Starting HEVC receiver on {}:{}...", bind_ip, port);
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
    println!("  vp-rcvr receive [--bind-ip IP] [--port N] [--payload N] [--clock-rate N] [--latency-ms N] [--no-preview] [--v4l2-device /dev/videoN]");
    println!();
    println!("Examples:");
    println!("  vp-rcvr receive --port 5000");
    println!("  vp-rcvr receive --port 5000 --v4l2-device /dev/video10");
    println!("  vp-rcvr receive --port 5000 --no-preview --v4l2-device /dev/video10");
}
