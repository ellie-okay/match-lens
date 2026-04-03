use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use tracing::{info, warn};

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSample {
    pub timestamp_sec: f64,
    pub target: String,
    pub rtt_ms: Option<f64>,
    pub timed_out: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
enum ProbeMethod {
    TcpConnect(SocketAddr),
    Icmp { host: String },
}

#[derive(Debug, Clone)]
struct ProbeTarget {
    sample_target: String,
    method: ProbeMethod,
}

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const TCP_PROBE_TIMEOUT: Duration = Duration::from_millis(1200);
const ICMP_PROBE_TIMEOUT_MS: u64 = 900;
const RIOT_TARGET_SCAN_INTERVAL: Duration = Duration::from_secs(5);

pub async fn poll_network_samples(
    recording_started_at: Instant,
    stop_flag: Arc<Mutex<bool>>,
) -> Vec<NetworkSample> {
    let control_target = ProbeTarget {
        sample_target: "control:1.1.1.1:443".to_string(),
        method: ProbeMethod::TcpConnect(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 443)),
    };
    let mut riot_target: Option<ProbeTarget> = None;
    let mut last_riot_scan: Option<Instant> = None;
    let mut samples = Vec::new();

    loop {
        if *stop_flag.lock().await {
            break;
        }

        if riot_target.is_none()
            && last_riot_scan
                .map(|last| last.elapsed() >= RIOT_TARGET_SCAN_INTERVAL)
                .unwrap_or(true)
        {
            last_riot_scan = Some(Instant::now());
            riot_target = detect_riot_target();
            if let Some(target) = &riot_target {
                info!("Detected Riot network probe target: {}", target.sample_target);
            }
        }

        let control_future = measure_target(recording_started_at, control_target.clone());
        let riot_future = async {
            match riot_target.clone() {
                Some(target) => Some(measure_target(recording_started_at, target).await),
                None => None,
            }
        };

        let (control_sample, riot_sample) = tokio::join!(control_future, riot_future);
        samples.push(control_sample);
        if let Some(sample) = riot_sample {
            samples.push(sample);
        }

        sleep(SAMPLE_INTERVAL).await;
    }

    samples
}

async fn measure_target(recording_started_at: Instant, target: ProbeTarget) -> NetworkSample {
    match target.method {
        ProbeMethod::TcpConnect(address) => measure_tcp_target(recording_started_at, target.sample_target, address).await,
        ProbeMethod::Icmp { host } => measure_icmp_target(recording_started_at, target.sample_target, &host).await,
    }
}

async fn measure_tcp_target(
    recording_started_at: Instant,
    target_label: String,
    address: SocketAddr,
) -> NetworkSample {
    let started = Instant::now();
    let result = timeout(TCP_PROBE_TIMEOUT, TcpStream::connect(address)).await;

    match result {
        Ok(Ok(stream)) => {
            drop(stream);
            NetworkSample {
                timestamp_sec: recording_started_at.elapsed().as_secs_f64(),
                target: target_label,
                rtt_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                timed_out: false,
                error: None,
            }
        }
        Ok(Err(e)) => NetworkSample {
            timestamp_sec: recording_started_at.elapsed().as_secs_f64(),
            target: target_label,
            rtt_ms: None,
            timed_out: false,
            error: Some(e.to_string()),
        },
        Err(_) => NetworkSample {
            timestamp_sec: recording_started_at.elapsed().as_secs_f64(),
            target: target_label,
            rtt_ms: None,
            timed_out: true,
            error: Some("timeout".to_string()),
        },
    }
}

async fn measure_icmp_target(
    recording_started_at: Instant,
    target_label: String,
    host: &str,
) -> NetworkSample {
    let started = Instant::now();
    let mut command = Command::new("ping");
    command
        .args([
            "-n",
            "1",
            "-w",
            &ICMP_PROBE_TIMEOUT_MS.to_string(),
            "-4",
            host,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);

    let output = match timeout(
        Duration::from_millis(ICMP_PROBE_TIMEOUT_MS + 500),
        command.output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return NetworkSample {
                timestamp_sec: recording_started_at.elapsed().as_secs_f64(),
                target: target_label,
                rtt_ms: None,
                timed_out: false,
                error: Some(e.to_string()),
            };
        }
        Err(_) => {
            return NetworkSample {
                timestamp_sec: recording_started_at.elapsed().as_secs_f64(),
                target: target_label,
                rtt_ms: None,
                timed_out: true,
                error: Some("ping command timeout".to_string()),
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(rtt_ms) = parse_ping_rtt_ms(&stdout) {
        return NetworkSample {
            timestamp_sec: recording_started_at.elapsed().as_secs_f64(),
            target: target_label,
            rtt_ms: Some(rtt_ms),
            timed_out: false,
            error: None,
        };
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let timed_out = stdout.contains("Request timed out")
        || stdout.contains("Destination host unreachable")
        || stdout.contains("General failure");
    let error_text = [stdout.trim(), stderr.trim()]
        .into_iter()
        .find(|text| !text.is_empty())
        .map(|text| text.to_string())
        .unwrap_or_else(|| "unparsed ping output".to_string());

    NetworkSample {
        timestamp_sec: recording_started_at.elapsed().as_secs_f64(),
        target: target_label,
        rtt_ms: if output.status.success() {
            Some(started.elapsed().as_secs_f64() * 1000.0)
        } else {
            None
        },
        timed_out,
        error: if output.status.success() && !timed_out {
            None
        } else {
            Some(error_text)
        },
    }
}

fn parse_ping_rtt_ms(output: &str) -> Option<f64> {
    for line in output.lines() {
        if let Some(index) = line.find("time=") {
            let suffix = &line[index + 5..];
            let digits: String = suffix
                .chars()
                .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
                .collect();
            if let Ok(value) = digits.parse::<f64>() {
                return Some(value);
            }
        }

        if line.contains("time<1ms") {
            return Some(0.5);
        }
    }

    None
}

fn detect_riot_target() -> Option<ProbeTarget> {
    for logs_root in league_gamelogs_roots() {
        match detect_riot_target_in_root(&logs_root) {
            Some(target) => return Some(target),
            None if logs_root.exists() => continue,
            None => {}
        }
    }

    None
}

fn detect_riot_target_in_root(root: &Path) -> Option<ProbeTarget> {
    let mut match_dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .map(|_| path)
        })
        .collect();

    sort_paths_by_modified_desc(&mut match_dirs);

    for match_dir in match_dirs.into_iter().take(5) {
        if let Some(target) = detect_riot_target_in_match_dir(&match_dir) {
            return Some(target);
        }
    }

    None
}

fn detect_riot_target_in_match_dir(match_dir: &Path) -> Option<ProbeTarget> {
    let mut log_files: Vec<PathBuf> = std::fs::read_dir(match_dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let is_r3dlog = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with("r3dlog.txt"))
                .unwrap_or(false);
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_file() && is_r3dlog)
                .map(|_| path)
        })
        .collect();

    sort_paths_by_modified_desc(&mut log_files);

    for log_file in log_files {
        match std::fs::read_to_string(&log_file) {
            Ok(contents) => {
                for line in contents.lines().rev() {
                    if let Some((ip, port)) = parse_riot_connection_line(line) {
                        return Some(ProbeTarget {
                            sample_target: format!("riot:{ip}:{port}"),
                            method: ProbeMethod::Icmp { host: ip },
                        });
                    }
                }
            }
            Err(e) => warn!("Could not read Riot game log {}: {e}", log_file.display()),
        }
    }

    None
}

fn parse_riot_connection_line(line: &str) -> Option<(String, u16)> {
    const ADDRESS_PREFIX: &str = "Connecting to address (";
    const PORT_PREFIX: &str = ") port (";

    let address_start = line.find(ADDRESS_PREFIX)? + ADDRESS_PREFIX.len();
    let address_rest = &line[address_start..];
    let address_end = address_rest.find(PORT_PREFIX)?;
    let ip = address_rest[..address_end].trim();
    let port_rest = &address_rest[address_end + PORT_PREFIX.len()..];
    let port_end = port_rest.find(')')?;
    let port = port_rest[..port_end].trim().parse::<u16>().ok()?;

    Some((ip.to_string(), port))
}

fn league_gamelogs_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(install_dir) = league_install_from_registry() {
        roots.push(PathBuf::from(install_dir).join("Logs").join("GameLogs"));
    }

    for root in &[
        r"C:\Riot Games",
        r"D:\Riot Games",
        r"C:\Program Files\Riot Games",
    ] {
        roots.push(
            PathBuf::from(root)
                .join("League of Legends")
                .join("Logs")
                .join("GameLogs"),
        );
    }

    roots.sort();
    roots.dedup();
    roots
}

fn league_install_from_registry() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    for key_path in &[
        r"SOFTWARE\WOW6432Node\Riot Games\League of Legends",
        r"SOFTWARE\Riot Games\League of Legends",
    ] {
        if let Ok(key) = hklm.open_subkey(key_path) {
            if let Ok(location) = key.get_value::<String, _>("Location") {
                return Some(location);
            }
        }
    }

    None
}

fn sort_paths_by_modified_desc(paths: &mut [PathBuf]) {
    paths.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    paths.reverse();
}
