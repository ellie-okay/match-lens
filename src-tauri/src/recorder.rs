use crate::audio::LoopbackCaptureSession;
use anyhow::{anyhow, Context, Result};
use std::fs::File;
use std::io::Write;
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use tracing::{info, warn};

const CREATE_NO_WINDOW: u32 = 0x08000000;
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MONITORINFOEXW, MonitorFromWindow,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible,
};

/// Return the pixel dimensions of the primary display only.
/// Uses Win32 GetSystemMetrics(SM_CXSCREEN/SM_CYSCREEN) which always returns the
/// primary monitor's size regardless of how many monitors are attached.
fn primary_monitor_size() -> (u32, u32) {
    #[link(name = "user32")]
    extern "system" {
        fn GetSystemMetrics(nIndex: i32) -> i32;
    }
    const SM_CXSCREEN: i32 = 0;
    const SM_CYSCREEN: i32 = 1;
    // SAFETY: GetSystemMetrics is a simple Win32 getter with no side-effects.
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    (w.max(1) as u32, h.max(1) as u32)
}

#[derive(Debug, Clone, Copy)]
struct CaptureTarget {
    monitor: HMONITOR,
    monitor_index: usize,
    offset_x: i32,
    offset_y: i32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy)]
enum CaptureBackend {
    WindowGraphics,
    DesktopDuplication,
    Gdi,
}

/// Build gfxcapture args targeting the primary monitor.
/// Monitor capture is more stable for League than binding to the game window,
/// which can change rendering state during launch/fullscreen transitions.
fn gfxcapture_input_args() -> Vec<String> {
    let target = detect_lol_capture_target();
    let source = match target {
        Some(target) => {
            info!(
                "Using LoL monitor handle for gfxcapture: {}",
                target.monitor.0 as usize
            );
            format!("hmonitor={}", target.monitor.0 as usize)
        }
        None => {
            warn!("Could not resolve LoL monitor handle, falling back to monitor_idx=0");
            "monitor_idx=0".to_string()
        }
    };

    vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("gfxcapture={source}:capture_cursor=1:max_framerate=60"),
    ]
}

fn detect_lol_capture_target() -> Option<CaptureTarget> {
    let foreground_hwnd = unsafe { GetForegroundWindow() };
    let mut search = WindowSearch {
        foreground_hwnd: if foreground_hwnd.0.is_null() {
            None
        } else {
            Some(foreground_hwnd)
        },
        ..Default::default()
    };
    let ptr = &mut search as *mut WindowSearch;

    let result = unsafe { EnumWindows(Some(enum_windows_find_lol_monitor), LPARAM(ptr as isize)) };
    if let Err(e) = result {
        warn!("EnumWindows failed while detecting LoL monitor: {e}");
    }

    let hwnd = search.best_hwnd?;
    capture_target_from_window(hwnd)
}

#[derive(Default)]
struct WindowSearch {
    best_hwnd: Option<HWND>,
    best_score: i64,
    foreground_hwnd: Option<HWND>,
}

unsafe extern "system" fn enum_windows_find_lol_monitor(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if !unsafe { IsWindowVisible(hwnd).as_bool() } || unsafe { IsIconic(hwnd).as_bool() } {
        return true.into();
    }

    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if process_id == 0 || !is_lol_process(process_id) {
        return true.into();
    }

    let Some(rect) = get_window_rect(hwnd) else {
        return true.into();
    };
    let width = i64::from(rect.right - rect.left);
    let height = i64::from(rect.bottom - rect.top);
    if width < 640 || height < 360 {
        return true.into();
    }

    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.0.is_null() {
        return true.into();
    }

    let Some(monitor_info) = get_monitor_info(monitor) else {
        return true.into();
    };
    let state = unsafe { &mut *(lparam.0 as *mut WindowSearch) };
    let monitor_rect = monitor_info.monitorInfo.rcMonitor;
    let monitor_width = i64::from(monitor_rect.right - monitor_rect.left).max(1);
    let monitor_height = i64::from(monitor_rect.bottom - monitor_rect.top).max(1);
    let monitor_area = monitor_width * monitor_height;
    let window_area = width * height;
    let coverage_pct = (window_area * 100) / monitor_area.max(1);
    let title = get_window_title(hwnd);
    let title_lower = title.to_ascii_lowercase();
    let title_bonus = if title_lower.contains("league of legends") {
        200_000_000i64
    } else {
        0
    };
    // Foreground bonus is intentionally small: during the loading-screen transition the
    // game window may briefly be foreground on the wrong monitor, and the foreground
    // heuristic should never override a strong coverage match on the correct monitor.
    let foreground_bonus = if Some(hwnd) == state.foreground_hwnd {
        100_000_000i64
    } else {
        0
    };
    let coverage_bonus = if coverage_pct >= 90 {
        2_000_000_000i64
    } else if coverage_pct >= 75 {
        800_000_000i64
    } else if coverage_pct >= 50 {
        200_000_000i64
    } else {
        0
    };
    let score = window_area + title_bonus + foreground_bonus + coverage_bonus;
    if score > state.best_score {
        state.best_score = score;
        state.best_hwnd = Some(hwnd);
    }

    true.into()
}

fn capture_target_from_window(hwnd: HWND) -> Option<CaptureTarget> {
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.0.is_null() {
        return None;
    }

    let monitor_info = get_monitor_info(monitor)?;
    let rect = monitor_info.monitorInfo.rcMonitor;
    let width = (rect.right - rect.left).max(1) as u32;
    let height = (rect.bottom - rect.top).max(1) as u32;
    let monitor_index = monitor_index_for_handle(monitor).unwrap_or(0);

    info!(
        "Resolved LoL capture target: hwnd={} monitor_index={} rect=({}, {}) {}x{} title={:?}",
        hwnd.0 as usize,
        monitor_index,
        rect.left,
        rect.top,
        width,
        height,
        get_window_title(hwnd)
    );

    Some(CaptureTarget {
        monitor,
        monitor_index,
        offset_x: rect.left,
        offset_y: rect.top,
        width,
        height,
    })
}

fn get_window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some(rect)
}

fn get_window_title(hwnd: HWND) -> String {
    let mut buffer = vec![0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..len as usize])
}

fn get_monitor_info(monitor: HMONITOR) -> Option<MONITORINFOEXW> {
    let mut monitor_info = MONITORINFOEXW::default();
    monitor_info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut monitor_info as *mut MONITORINFOEXW as *mut MONITORINFO) }
        .as_bool()
    {
        Some(monitor_info)
    } else {
        None
    }
}

fn monitor_index_for_handle(target: HMONITOR) -> Option<usize> {
    #[derive(Default)]
    struct MonitorSearch {
        target: isize,
        current_index: usize,
        found_index: Option<usize>,
    }

    unsafe extern "system" fn enum_monitor(
        hmonitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let state = unsafe { &mut *(lparam.0 as *mut MonitorSearch) };
        if hmonitor.0 == state.target as *mut _ {
            state.found_index = Some(state.current_index);
            return false.into();
        }
        state.current_index += 1;
        true.into()
    }

    let mut search = MonitorSearch {
        target: target.0 as isize,
        ..Default::default()
    };
    let ptr = &mut search as *mut MonitorSearch;
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(enum_monitor), LPARAM(ptr as isize)) };
    search.found_index
}

fn is_lol_process(process_id: u32) -> bool {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok();
    let Some(handle) = handle else {
        return false;
    };

    let image_name = query_process_image_name(handle);
    let _ = unsafe { CloseHandle(handle) };

    match image_name {
        Some(path) => path
            .rsplit(['\\', '/'])
            .next()
            .map(|name| name.eq_ignore_ascii_case("League of Legends.exe"))
            .unwrap_or(false),
        None => false,
    }
}

fn query_process_image_name(handle: HANDLE) -> Option<String> {
    let mut buffer = vec![0u16; 1024];
    let mut size = buffer.len() as u32;

    if unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    }
    .is_err()
    {
        return None;
    }

    Some(String::from_utf16_lossy(&buffer[..size as usize]))
}

/// Build ddagrab input args scoped to the primary monitor.
/// Desktop Duplication is less invasive than gdigrab and captures the cursor
/// through the compositor path, which is a better fit for modern games.
fn ddagrab_input_args() -> Vec<String> {
    let target = detect_lol_capture_target();
    let (output_idx, w, h) = if let Some(target) = target {
        info!(
            "Using LoL monitor for ddagrab: output_idx={} {}x{}",
            target.monitor_index, target.width, target.height
        );
        (target.monitor_index, target.width, target.height)
    } else {
        let (w, h) = primary_monitor_size();
        info!("Primary monitor for ddagrab: {}x{}", w, h);
        (0usize, w, h)
    };
    vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!(
            "ddagrab=output_idx={}:framerate=60:draw_mouse=1:video_size={}x{}",
            output_idx, w, h
        ),
    ]
}

/// Build gdigrab input args scoped to the primary monitor only.
/// This stays as a fallback for systems where Desktop Duplication is unavailable.
fn gdigrab_input_args() -> Vec<String> {
    let target = detect_lol_capture_target();
    let (offset_x, offset_y, w, h) = if let Some(target) = target {
        info!(
            "Using LoL monitor for gdigrab: offset=({}, {}) {}x{}",
            target.offset_x, target.offset_y, target.width, target.height
        );
        (target.offset_x, target.offset_y, target.width, target.height)
    } else {
        let (w, h) = primary_monitor_size();
        info!("Primary monitor: {}x{}", w, h);
        (0, 0, w, h)
    };
    vec![
        "-f".into(),
        "gdigrab".into(),
        "-framerate".into(),
        "60".into(),
        "-draw_mouse".into(),
        "1".into(),
        "-offset_x".into(),
        offset_x.to_string(),
        "-offset_y".into(),
        offset_y.to_string(),
        "-video_size".into(),
        format!("{}x{}", w, h),
        "-i".into(),
        "desktop".into(),
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub enum Encoder {
    Nvenc,
    Amf,
    Qsv,
    Software,
}

pub struct RecordingSession {
    process: Child,
    stdin: Option<ChildStdin>,
    video_output_path: PathBuf,
    audio_session: Option<LoopbackCaptureSession>,
    pub output_path: PathBuf,
}

impl RecordingSession {
    /// Gracefully stop FFmpeg by writing 'q' to stdin, then wait for it to finalize.
    pub fn stop(mut self) -> Result<PathBuf> {
        info!("Stopping FFmpeg: {:?}", self.output_path);
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.write_all(b"q");
            let _ = stdin.flush();
        }

        let status = self.process.wait().context("Failed to wait for FFmpeg")?;
        if !status.success() {
            warn!("FFmpeg exited with status {}", status);
        }

        if let Some(audio_session) = self.audio_session.take() {
            let audio_path = audio_session
                .stop()
                .context("Failed to stop native loopback audio capture")?;

            match mux_audio_into_video(&self.video_output_path, &audio_path, &self.output_path) {
                Ok(()) => {
                    cleanup_temp_file(&self.video_output_path);
                    cleanup_temp_file(&audio_path);
                    info!("Recording finalized with audio: {:?}", self.output_path);
                    return Ok(self.output_path);
                }
                Err(e) => {
                    warn!("Audio mux failed, keeping video-only recording: {e}");
                    cleanup_temp_file(&audio_path);
                    return keep_video_only_output(&self.video_output_path, &self.output_path);
                }
            }
        }

        info!("Recording finalized: {:?}", self.output_path);
        Ok(self.output_path)
    }
}

fn capture_backends() -> [CaptureBackend; 3] {
    [
        CaptureBackend::WindowGraphics,
        CaptureBackend::Gdi,
        CaptureBackend::DesktopDuplication,
    ]
}

/// Detect available hardware encoder.
pub fn detect_encoder() -> Encoder {
    let ffmpeg = ffmpeg_binary_path();
    let output = Command::new(&ffmpeg)
        .args(["-encoders", "-v", "quiet"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            if text.contains("h264_nvenc") {
                info!("Hardware encoder: NVENC");
                Encoder::Nvenc
            } else if text.contains("h264_amf") {
                info!("Hardware encoder: AMF");
                Encoder::Amf
            } else if text.contains("h264_qsv") {
                info!("Hardware encoder: QuickSync");
                Encoder::Qsv
            } else {
                warn!("No HW encoder found, using libx264");
                Encoder::Software
            }
        }
        Err(e) => {
            warn!("FFmpeg probe failed: {e}");
            Encoder::Software
        }
    }
}

/// Record primary display plus native Windows loopback audio capture.
pub fn start_recording_with_system_audio(
    output_path: &PathBuf,
    resolution: &str,
) -> Result<RecordingSession> {
    let video_output_path = temp_media_path(output_path, "video", "mp4");
    let audio_output_path = temp_media_path(output_path, "audio", "wav");

    cleanup_temp_file(&video_output_path);
    cleanup_temp_file(&audio_output_path);

    let audio_session = LoopbackCaptureSession::start(&audio_output_path)
        .context("Failed to start native loopback audio capture")?;
    info!(
        "Native loopback audio temp file: {:?}",
        audio_session.output_path()
    );

    match spawn_video_capture(&video_output_path, resolution) {
        Ok(mut session) => {
            session.audio_session = Some(audio_session);
            session.output_path = output_path.clone();
            Ok(session)
        }
        Err(e) => {
            let _ = audio_session.stop();
            cleanup_temp_file(&audio_output_path);
            Err(e)
        }
    }
}

/// Record primary display with no audio.
pub fn start_recording(
    output_path: &PathBuf,
    resolution: &str,
    _audio_mode: &str,
) -> Result<RecordingSession> {
    spawn_video_capture(output_path, resolution)
}

fn video_encode_args(encoder: &Encoder) -> Vec<String> {
    match encoder {
        Encoder::Nvenc => vec![
            "-c:v".into(),
            "h264_nvenc".into(),
            "-preset".into(),
            "p4".into(),
            "-b:v".into(),
            "8M".into(),
        ],
        Encoder::Amf => vec![
            "-c:v".into(),
            "h264_amf".into(),
            "-quality".into(),
            "speed".into(),
            "-b:v".into(),
            "8M".into(),
        ],
        Encoder::Qsv => vec![
            "-c:v".into(),
            "h264_qsv".into(),
            "-preset".into(),
            "faster".into(),
            "-b:v".into(),
            "8M".into(),
        ],
        Encoder::Software => vec![
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "veryfast".into(),
            "-crf".into(),
            "23".into(),
        ],
    }
}

fn spawn_video_capture(output_path: &Path, resolution: &str) -> Result<RecordingSession> {
    let encoder = detect_encoder();
    let mut last_error: Option<anyhow::Error> = None;

    for backend in capture_backends() {
        let mut args: Vec<String> = vec!["-loglevel".into(), "warning".into()];
        args.extend(capture_input_args(backend));
        args.extend(video_encode_args(&encoder));
        args.extend(["-vf".into(), capture_filter_chain(backend, resolution)]);
        args.extend(["-an".into()]);
        args.extend(["-movflags".into(), "+faststart".into()]);
        args.push(output_path.to_string_lossy().to_string());

        match spawn_ffmpeg(args, output_path.to_path_buf()) {
            Ok(session) => {
                info!("Video capture backend active: {:?}", backend);
                return Ok(session);
            }
            Err(e) => {
                warn!("Video capture backend {:?} failed: {e}", backend);
                cleanup_temp_file(output_path);
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("No video capture backend succeeded")))
}

fn capture_input_args(backend: CaptureBackend) -> Vec<String> {
    match backend {
        CaptureBackend::WindowGraphics => gfxcapture_input_args(),
        CaptureBackend::DesktopDuplication => ddagrab_input_args(),
        CaptureBackend::Gdi => gdigrab_input_args(),
    }
}

fn capture_filter_chain(backend: CaptureBackend, resolution: &str) -> String {
    let scale = if resolution == "1080p" {
        "scale=1920:1080:flags=lanczos,format=yuv420p"
    } else {
        "format=yuv420p"
    };

    match backend {
        CaptureBackend::WindowGraphics => {
            format!("hwdownload,format=bgra,{scale}")
        }
        CaptureBackend::DesktopDuplication => {
            format!("hwdownload,format=bgra,{scale}")
        }
        CaptureBackend::Gdi => scale.to_string(),
    }
}

fn spawn_ffmpeg(args: Vec<String>, output_path: PathBuf) -> Result<RecordingSession> {
    let ffmpeg = ffmpeg_binary_path();
    let log_path = ffmpeg_log_path();
    let stderr_file = File::create(&log_path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());

    info!("FFmpeg command: {:?} {}", ffmpeg, args.join(" "));
    info!("FFmpeg stderr log: {:?}", log_path);

    let mut child = Command::new(&ffmpeg)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(stderr_file)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn ffmpeg ({ffmpeg:?}): {e}"))?;

    std::thread::sleep(std::time::Duration::from_millis(800));
    match child.try_wait() {
        Ok(Some(status)) => {
            return Err(anyhow!(
                "FFmpeg exited immediately ({}); check ffmpeg.log",
                status
            ));
        }
        Ok(None) => {}
        Err(e) => warn!("Could not poll FFmpeg status: {e}"),
    }

    let stdin = child.stdin.take();
    info!("FFmpeg running -> {:?}", output_path);

    Ok(RecordingSession {
        process: child,
        stdin,
        video_output_path: output_path.clone(),
        audio_session: None,
        output_path,
    })
}

fn mux_audio_into_video(video_path: &Path, audio_path: &Path, output_path: &Path) -> Result<()> {
    let ffmpeg = ffmpeg_binary_path();
    let log_path = ffmpeg_log_path();
    let stderr_file = File::create(&log_path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());

    let args = vec![
        "-y".into(),
        "-loglevel".into(),
        "warning".into(),
        "-i".into(),
        video_path.to_string_lossy().to_string(),
        "-i".into(),
        audio_path.to_string_lossy().to_string(),
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "1:a:0".into(),
        "-c:v".into(),
        "copy".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "192k".into(),
        "-af".into(),
        "apad".into(),
        "-shortest".into(),
        "-movflags".into(),
        "+faststart".into(),
        output_path.to_string_lossy().to_string(),
    ];

    info!(
        "Muxing temp audio/video into final output: {:?} + {:?} -> {:?}",
        video_path, audio_path, output_path
    );

    let status = Command::new(&ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr_file)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| anyhow!("Failed to spawn ffmpeg mux step ({ffmpeg:?}): {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("FFmpeg mux step failed with status {}", status))
    }
}

fn keep_video_only_output(video_path: &Path, output_path: &Path) -> Result<PathBuf> {
    if video_path == output_path {
        return Ok(output_path.to_path_buf());
    }

    if output_path.exists() {
        std::fs::remove_file(output_path).with_context(|| {
            format!(
                "Failed to remove existing output before fallback rename: {}",
                output_path.display()
            )
        })?;
    }

    match std::fs::rename(video_path, output_path) {
        Ok(()) => {}
        Err(rename_err) => {
            std::fs::copy(video_path, output_path)
                .map(|_| ())
                .with_context(|| {
                    format!(
                        "Failed to copy video-only fallback after rename failure ({rename_err}): {} -> {}",
                        video_path.display(),
                        output_path.display()
                    )
                })?;
            let _ = std::fs::remove_file(video_path);
        }
    }

    Ok(output_path.to_path_buf())
}

fn ffmpeg_log_path() -> PathBuf {
    let log_path = dirs::config_dir()
        .unwrap_or_default()
        .join("lol-review")
        .join("ffmpeg.log");

    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    log_path
}

fn cleanup_temp_file(path: &Path) {
    if path.exists() {
        if let Err(e) = std::fs::remove_file(path) {
            warn!("Could not delete temp file {}: {e}", path.display());
        }
    }
}

fn temp_media_path(final_output_path: &Path, suffix: &str, extension: &str) -> PathBuf {
    let parent = final_output_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = final_output_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("recording");

    parent.join(format!("{stem}.{suffix}.{extension}"))
}

/// Resolve the bundled ffmpeg binary path.
fn ffmpeg_binary_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let exe_dir = exe.parent().unwrap_or(std::path::Path::new("."));

        let sidecar = exe_dir.join("ffmpeg-x86_64-pc-windows-msvc.exe");
        if sidecar.exists() {
            return sidecar;
        }

        for ancestor in exe_dir.ancestors() {
            let candidate = ancestor
                .join("src-tauri")
                .join("binaries")
                .join("ffmpeg-x86_64-pc-windows-msvc.exe");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    PathBuf::from("ffmpeg")
}

/// Build the output file path for a new recording.
pub fn build_output_path(output_dir: &str, champion: &str, result: &str) -> Result<PathBuf> {
    let dir = if output_dir.is_empty() {
        dirs::video_dir().unwrap_or_default().join("LoL Recordings")
    } else {
        PathBuf::from(output_dir)
    };
    std::fs::create_dir_all(&dir)?;
    let ts = chrono::Local::now().format("%Y-%m-%d_%H%M%S");
    Ok(dir.join(format!("{}_{}_{}.mp4", ts, champion, result)))
}
