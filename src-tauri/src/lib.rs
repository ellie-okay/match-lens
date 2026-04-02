mod audio;
mod config;
mod db;
mod events;
mod lcu;
mod network;
mod recorder;
mod storage;

use chrono::Local;
use reqwest::Client;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
};

// ─── App State ────────────────────────────────────────────────────────────────

pub struct ActiveRecording {
    pub start_time: std::time::Instant,
    pub output_path: String,
    /// Signal to the blocking FFmpeg thread to stop (std channel works across sync/async boundary)
    pub stop_tx: std::sync::mpsc::Sender<()>,
    /// Receives completion from the blocking recorder thread once mux/finalize is done
    pub recorder_done_rx: tokio::sync::oneshot::Receiver<Result<String, String>>,
    /// Signal to tell background pollers to stop
    pub background_stop: Arc<Mutex<bool>>,
    /// Receives the final collected events when polling stops
    pub events_rx: tokio::sync::oneshot::Receiver<Vec<events::GameEvent>>,
    /// Receives the final collected network samples when polling stops
    pub network_rx: tokio::sync::oneshot::Receiver<Vec<network::NetworkSample>>,
}

#[derive(Default)]
pub struct AppState {
    pub recording: Mutex<Option<ActiveRecording>>,
    pub summoner_name: Mutex<String>,
    pub lcu_creds: Mutex<Option<lcu::LcuCredentials>>,
}

// ─── Tauri Commands ────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_matches() -> Result<Vec<db::Match>, String> {
    tokio::task::spawn_blocking(|| {
        let conn = db::open().map_err(|e| e.to_string())?;
        db::get_matches(&conn).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_events(match_id: i64) -> Result<Vec<db::Event>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = db::open().map_err(|e| e.to_string())?;
        db::get_events(&conn, match_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_network_samples(match_id: i64) -> Result<Vec<db::NetworkSample>, String> {
    tokio::task::spawn_blocking(move || {
        let conn = db::open().map_err(|e| e.to_string())?;
        db::get_network_samples(&conn, match_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_settings() -> Result<config::Config, String> {
    Ok(config::load())
}

#[tauri::command]
async fn save_settings(cfg: config::Config) -> Result<(), String> {
    let autostart = cfg.app.autostart;
    config::save(&cfg).map_err(|e| e.to_string())?;
    if let Err(e) = set_autostart(autostart) {
        warn!("Failed to set autostart: {e}");
    }
    Ok(())
}

#[tauri::command]
async fn get_recording_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let rec = state.recording.lock().await;
    if let Some(r) = &*rec {
        Ok(serde_json::json!({
            "recording": true,
            "output_path": r.output_path,
            "elapsed_secs": r.start_time.elapsed().as_secs()
        }))
    } else {
        Ok(serde_json::json!({ "recording": false }))
    }
}

#[tauri::command]
async fn delete_recording(match_id: i64) -> Result<(), String> {
    info!("delete_recording: match_id={match_id}");
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let conn = db::open().map_err(|e| e.to_string())?;
        // Query video path directly by ID — avoids "Match not found" if list is stale.
        let video_path: Option<String> = conn
            .query_row(
                "SELECT video_path FROM matches WHERE id = ?1",
                rusqlite::params![match_id],
                |row| row.get(0),
            )
            .ok();
        info!("delete_recording: video_path={video_path:?}");
        // Delete the file; non-fatal if missing or already gone.
        if let Some(ref path) = video_path {
            if let Err(e) = storage::delete_video_file(path) {
                warn!("Could not delete video file {path}: {e}");
            }
        }
        db::delete_match(&conn, match_id).map_err(|e| e.to_string())?;
        info!("delete_recording: match {match_id} deleted from DB");
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn open_recordings_folder() -> Result<(), String> {
    let cfg = config::load();
    let path = if cfg.recording.output_dir.is_empty() {
        dirs::video_dir()
            .unwrap_or_default()
            .join("Match Lens Recordings")
            .to_string_lossy()
            .to_string()
    } else {
        cfg.recording.output_dir.clone()
    };
    std::fs::create_dir_all(&path).ok();
    open_in_explorer(&path);
    Ok(())
}

#[tauri::command]
async fn get_storage_usage() -> Result<serde_json::Value, String> {
    let cfg = config::load();
    let (used, max) = storage::storage_usage(&cfg.recording.output_dir, cfg.storage.max_gb);
    Ok(serde_json::json!({ "used_gb": used, "max_gb": max }))
}

// ─── System Tray ──────────────────────────────────────────────────────────────

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open_review = MenuItem::with_id(app, "open_review", "Open Review", true, None::<&str>)?;
    let open_settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&open_review, &open_settings, &quit])?;

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open_review" => show_review_window(app),
            "settings" => show_settings_window(app),
            "quit" => {
                info!("Quit via tray");
                // Destroy all windows before exit so WebView2 can unregister its
                // window classes cleanly (avoids Chrome_WidgetWin_0 unregister error).
                for (_, w) in app.webview_windows() {
                    let _ = w.destroy();
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick { .. } = event {
                show_review_window(tray.app_handle());
            }
        })
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Match Lens")
        .build(app)?;

    Ok(())
}

fn show_review_window(app: &AppHandle) {
    // Windows are created on-demand so we always have the correct URL.
    // If a hidden window already exists, restore it without stealing focus.
    if let Some(window) = app.get_webview_window("review") {
        apply_review_window_layout(&window);
        show_review_window_no_focus(&window);
        let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
        return;
    }
    if let Ok(w) = WebviewWindowBuilder::new(app, "review", WebviewUrl::App("/".into()))
        .title("Match Lens")
        .inner_size(1280.0, 720.0)
        .min_inner_size(900.0, 550.0)
        .visible(false)
        .focused(false)
        .theme(Some(tauri::Theme::Dark))
        .build()
    {
        attach_close_to_tray(&w);
        attach_review_window_tracking(&w);
        apply_review_window_layout(&w);
        show_review_window_no_focus(&w);
        let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
}

fn show_settings_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    if let Ok(w) = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("/settings".into()))
        .title("Settings – Match Lens")
        .inner_size(480.0, 520.0)
        .resizable(false)
        .theme(Some(tauri::Theme::Dark))
        .build()
    {
        attach_close_to_tray(&w);
    }
}

/// Intercept the close button so it hides the window instead of exiting.
fn attach_close_to_tray(window: &tauri::WebviewWindow) {
    let w = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = w.hide();
        }
    });
}

fn attach_review_window_tracking(window: &tauri::WebviewWindow) {
    let w = window.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::CloseRequested { .. } => {
            persist_review_window_monitor(&w);
        }
        _ => {}
    });
}

fn apply_review_window_layout(window: &WebviewWindow) {
    if let Some(target_monitor) = resolve_saved_review_monitor(window) {
        let current_monitor = window.current_monitor().ok().flatten();
        if !same_monitor(current_monitor.as_ref(), Some(&target_monitor)) {
            let _ = window.unmaximize();
            let position = target_monitor.position();
            let _ = window.set_position(PhysicalPosition::new(position.x, position.y));
        }
    }

    let _ = window.maximize();
}

fn resolve_saved_review_monitor(window: &WebviewWindow) -> Option<tauri::Monitor> {
    let saved = config::load().app.review_window;
    let monitors = window.available_monitors().ok()?;

    if !saved.monitor_name.is_empty() {
        if let Some(monitor) = monitors
            .iter()
            .find(|monitor| monitor.name().is_some_and(|name| name == &saved.monitor_name))
        {
            return Some(monitor.clone());
        }
    }

    match (saved.monitor_x, saved.monitor_y) {
        (Some(x), Some(y)) => monitors
            .iter()
            .find(|monitor| {
                let position = monitor.position();
                position.x == x && position.y == y
            })
            .cloned(),
        _ => None,
    }
}

fn same_monitor(left: Option<&tauri::Monitor>, right: Option<&tauri::Monitor>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            if let (Some(left_name), Some(right_name)) = (left.name(), right.name()) {
                return left_name == right_name;
            }

            let left_position = left.position();
            let right_position = right.position();
            left_position.x == right_position.x && left_position.y == right_position.y
        }
        (None, None) => true,
        _ => false,
    }
}

fn persist_review_window_monitor(window: &WebviewWindow) {
    let monitor = match window.current_monitor() {
        Ok(Some(monitor)) => monitor,
        Ok(None) => return,
        Err(e) => {
            warn!("Could not read review window monitor: {e}");
            return;
        }
    };

    let mut cfg = config::load();
    let position = monitor.position();
    let monitor_name = monitor.name().cloned().unwrap_or_default();
    let review_window = &mut cfg.app.review_window;

    if review_window.monitor_name == monitor_name
        && review_window.monitor_x == Some(position.x)
        && review_window.monitor_y == Some(position.y)
    {
        return;
    }

    review_window.monitor_name = monitor_name;
    review_window.monitor_x = Some(position.x);
    review_window.monitor_y = Some(position.y);

    if let Err(e) = config::save(&cfg) {
        warn!("Failed to persist review window monitor: {e}");
    }
}

fn show_review_window_no_focus(window: &WebviewWindow) {
    let was_minimized = window.is_minimized().unwrap_or(false);
    if was_minimized {
        let _ = window.unminimize();
    }
    let _ = window.show();
    let _ = window.maximize();
    bring_window_to_front_no_focus(window);
}

#[cfg(target_os = "windows")]
fn bring_window_to_front_no_focus(window: &WebviewWindow) {
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn bring_window_to_front_no_focus(_window: &WebviewWindow) {}

// ─── Autostart ────────────────────────────────────────────────────────────────

fn set_autostart(enable: bool) -> anyhow::Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = hkcu.open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        KEY_SET_VALUE,
    )?;

    if enable {
        let exe_path = std::env::current_exe()?;
        run_key.set_value("MatchLens", &exe_path.to_string_lossy().as_ref())?;
    } else {
        let _ = run_key.delete_value("MatchLens");
    }

    Ok(())
}

// ─── Recording Orchestration ──────────────────────────────────────────────────

/// Main loop: watch LCU game phase and manage recording lifecycle.
async fn run_recording_loop(app: AppHandle) {
    let (phase_tx, mut phase_rx) = mpsc::channel::<lcu::GamePhase>(16);
    tokio::spawn(lcu::watch_game_phase(phase_tx));

    let mut current_phase = lcu::GamePhase::None;

    loop {
        let phase = match phase_rx.recv().await {
            Some(p) => p,
            None => break,
        };

        if phase == current_phase {
            continue;
        }

        let prev = current_phase.clone();
        current_phase = phase.clone();

        if phase == lcu::GamePhase::InProgress && prev != lcu::GamePhase::InProgress {
            info!("Game started → start recording");
            on_game_start(&app).await;
        } else if phase != lcu::GamePhase::InProgress && prev == lcu::GamePhase::InProgress {
            // Covers WaitingForStats, EndOfGame, PreEndOfGame, and also None
            // (Practice Tool exits directly InProgress → None with no end-of-game screen)
            info!("Game ended (phase={:?}) → stop recording", phase);
            on_game_end(&app).await;
        }
    }
}

async fn on_game_start(app: &AppHandle) {
    let state = app.state::<AppState>();

    pause_and_minimize_review_window(app);

    if state.recording.lock().await.is_some() {
        warn!("on_game_start called but already recording");
        return;
    }

    let cfg = config::load();

    // Wait for the game window to finish its fullscreen transition before detecting the
    // capture monitor and starting recording. At the moment InProgress is detected the
    // game window may not yet be positioned on the correct monitor.
    const WINDOW_SETTLE_SECS: u64 = 4;
    info!("Waiting {WINDOW_SETTLE_SECS}s for game window to settle");
    tokio::time::sleep(std::time::Duration::from_secs(WINDOW_SETTLE_SECS)).await;

    // Get summoner name from in-memory state (kept current by the LCU refresher).
    // Fall back to the Live Game API, which is available once the game has loaded.
    let summoner_name = {
        let name = state.summoner_name.lock().await.clone();
        if !name.is_empty() {
            name
        } else {
            let client = Client::builder()
                .danger_accept_invalid_certs(true)
                .build()
                .unwrap_or_else(|_| Client::new());
            match lcu::fetch_active_game_summoner(&client).await {
                Ok(n) => {
                    *state.summoner_name.lock().await = n.clone();
                    n
                }
                Err(e) => {
                    debug!("Could not detect summoner name from live game API: {e}");
                    String::new()
                }
            }
        }
    };

    // Build output path (temp name until we know champion)
    let output_path =
        match recorder::build_output_path(&cfg.recording.output_dir, "Recording", "InProgress") {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to build output path: {e}");
                return;
            }
        };

    let path_str = output_path.to_string_lossy().to_string();

    // Spawn the recorder in a blocking thread and wait for startup confirmation
    // before advertising an active recording to the rest of the app.
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let (recorder_done_tx, recorder_done_rx) =
        tokio::sync::oneshot::channel::<Result<String, String>>();
    let output_path_clone = output_path.clone();
    let resolution = cfg.recording.resolution.clone();
    let audio_mode = cfg.recording.audio_mode.clone();

    tokio::task::spawn_blocking(move || {
        let session_result = if audio_mode == "off" {
            recorder::start_recording(&output_path_clone, &resolution, "off")
        } else {
            recorder::start_recording_with_system_audio(&output_path_clone, &resolution)
        };

        match session_result {
            Ok(session) => {
                let _ = started_tx.send(Ok(()));
                info!("FFmpeg recording started");
                // Block this thread until stop signal arrives (or sender is dropped)
                let _ = stop_rx.recv();
                match session.stop() {
                    Ok(p) => {
                        info!("FFmpeg stopped: {:?}", p);
                        let _ = recorder_done_tx.send(Ok(p.to_string_lossy().to_string()));
                    }
                    Err(e) => {
                        error!("FFmpeg stop error: {e}");
                        let _ = recorder_done_tx.send(Err(e.to_string()));
                    }
                }
            }
            Err(e) => {
                let message = e.to_string();
                let _ = started_tx.send(Err(message.clone()));
                error!("Failed to start recording: {message}");
                let _ = recorder_done_tx.send(Err(message));
            }
        }
    });

    match tokio::time::timeout(std::time::Duration::from_secs(5), started_rx).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => {
            error!("Recording startup failed: {e}");
            return;
        }
        Ok(Err(_)) => {
            error!("Recording startup channel closed before initialization finished");
            return;
        }
        Err(_) => {
            error!("Timed out waiting for recording startup");
            return;
        }
    }

    // Spawn event poller
    let background_stop = Arc::new(Mutex::new(false));
    let events_stop_clone = Arc::clone(&background_stop);
    let summoner_clone = summoner_name.clone();
    let (events_result_tx, events_result_rx) = tokio::sync::oneshot::channel();
    let network_stop_clone = Arc::clone(&background_stop);
    let (network_result_tx, network_result_rx) = tokio::sync::oneshot::channel();

    let recording_started_at = std::time::Instant::now();

    tokio::spawn(async move {
        let collected =
            events::poll_events(summoner_clone, events_stop_clone, recording_started_at).await;
        let _ = events_result_tx.send(collected);
    });

    tokio::spawn(async move {
        let samples = network::poll_network_samples(recording_started_at, network_stop_clone).await;
        let _ = network_result_tx.send(samples);
    });

    *state.recording.lock().await = Some(ActiveRecording {
        start_time: recording_started_at,
        output_path: path_str,
        stop_tx,
        recorder_done_rx,
        background_stop,
        events_rx: events_result_rx,
        network_rx: network_result_rx,
    });

    info!("Recording session started");
}

fn pause_and_minimize_review_window(app: &AppHandle) {
    let _ = app.emit("game-started", serde_json::json!({}));

    if let Some(window) = app.get_webview_window("review") {
        if let Err(e) = window.minimize() {
            warn!("Failed to minimize review window on game start: {e}");
        }
    }
}

async fn on_game_end(app: &AppHandle) {
    let state = app.state::<AppState>();

    let active = state.recording.lock().await.take();
    let active = match active {
        Some(a) => a,
        None => {
            info!("on_game_end: no active recording");
            return;
        }
    };

    // Signal event poller to stop
    *active.background_stop.lock().await = true;

    // Signal FFmpeg blocking thread to stop
    let _ = active.stop_tx.send(());

    let app_clone = app.clone();
    tokio::spawn(async move {
        let output_path = match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            active.recorder_done_rx,
        )
        .await
        {
            Ok(Ok(Ok(path))) => path,
            Ok(Ok(Err(e))) => {
                error!("Recorder finalize failed: {e}");
                active.output_path.clone()
            }
            Ok(Err(_)) => {
                error!("Recorder finalize channel closed before completion");
                active.output_path.clone()
            }
            Err(_) => {
                error!("Timed out waiting for recorder finalize");
                active.output_path.clone()
            }
        };

        // Collect events (with timeout)
        let collected_events =
            tokio::time::timeout(std::time::Duration::from_secs(5), active.events_rx)
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or_default();

        let network_samples =
            tokio::time::timeout(std::time::Duration::from_secs(5), active.network_rx)
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or_default();

        finalize_recording(app_clone, output_path, collected_events, network_samples).await;
    });
}

async fn finalize_recording(
    app: AppHandle,
    output_path: String,
    collected_events: Vec<events::GameEvent>,
    network_samples: Vec<network::NetworkSample>,
) {
    let cfg = config::load();
    let state = app.state::<AppState>();

    let file_size = std::fs::metadata(&output_path)
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    // Fetch match summary from LCU
    let creds_opt = state.lcu_creds.lock().await.clone();
    let summoner_name = state.summoner_name.lock().await.clone();

    let (champion, kills, deaths, assists, result, duration_sec, game_id) =
        if let Some(ref creds) = creds_opt {
            let client = lcu::build_lcu_client().unwrap_or_else(|_| Client::new());
            match lcu::fetch_match_summary(&client, creds, &summoner_name).await {
                Ok(s) => (
                    s.champion,
                    s.kills,
                    s.deaths,
                    s.assists,
                    s.result,
                    s.duration_sec,
                    s.game_id,
                ),
                Err(e) => {
                    warn!("Could not fetch match summary: {e}");
                    (
                        "Unknown".into(),
                        0,
                        0,
                        0,
                        "Unknown".into(),
                        0,
                        uuid_fallback(),
                    )
                }
            }
        } else {
            (
                "Unknown".into(),
                0,
                0,
                0,
                "Unknown".into(),
                0,
                uuid_fallback(),
            )
        };

    // Rename the temp file to the proper name
    let final_path = rename_recording(&output_path, &champion, &result, &cfg.recording.output_dir);

    // Write match to DB
    let match_record = db::Match {
        id: 0,
        game_id: game_id.clone(),
        summoner_name: summoner_name.clone(),
        champion: champion.clone(),
        kills,
        deaths,
        assists,
        result: result.clone(),
        duration_sec,
        recorded_at: Local::now().to_rfc3339(),
        video_path: final_path.clone(),
        file_size_bytes: file_size,
    };

    let match_id = match tokio::task::spawn_blocking({
        let m = match_record.clone();
        move || -> anyhow::Result<i64> {
            let conn = db::open()?;
            let id = db::insert_match(&conn, &m)?;
            Ok(id)
        }
    })
    .await
    {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => {
            error!("DB insert_match failed: {e}");
            return;
        }
        Err(e) => {
            error!("DB task panicked: {e}");
            return;
        }
    };

    // Write events to DB
    if !collected_events.is_empty() {
        let events_to_insert = collected_events.clone();
        let _ = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = db::open()?;
            for ev in &events_to_insert {
                let db_ev = db::Event {
                    id: 0,
                    match_id,
                    event_type: ev.event_type.clone(),
                    timestamp_sec: ev.timestamp_sec,
                    is_local_player: ev.is_local_player,
                    raw_data: ev.raw_data.clone(),
                };
                db::insert_event(&conn, &db_ev)?;
            }
            Ok(())
        })
        .await;
    }

    if !network_samples.is_empty() {
        let samples_to_insert = network_samples.clone();
        let _ = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = db::open()?;
            for sample in &samples_to_insert {
                let db_sample = db::NetworkSample {
                    id: 0,
                    match_id,
                    timestamp_sec: sample.timestamp_sec,
                    target: sample.target.clone(),
                    rtt_ms: sample.rtt_ms,
                    timed_out: sample.timed_out,
                    error: sample.error.clone(),
                };
                db::insert_network_sample(&conn, &db_sample)?;
            }
            Ok(())
        })
        .await;
    }

    // Enforce storage limit
    enforce_storage_limit(&app, &cfg).await;

    // Open the review window first so its JS listener is live before we send the event.
    // If the window was hidden, WebView2 suspends JS execution — emitting while it's
    // hidden means the "game-recorded" listener never fires and the new game doesn't appear.
    show_review_window(&app);

    // Give the window ~400 ms to become visible and register its Tauri event listener.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let _ = app.emit(
        "game-recorded",
        serde_json::json!({
            "match_id": match_id,
            "champion": champion,
            "result": result,
            "kills": kills,
            "deaths": deaths,
            "assists": assists,
        }),
    );

    info!("Finalized: {champion} {result} {kills}/{deaths}/{assists} (match_id={match_id})");
}

async fn enforce_storage_limit(app: &AppHandle, cfg: &config::Config) {
    if !storage::is_over_limit(&cfg.recording.output_dir, cfg.storage.max_gb) {
        return;
    }

    info!(
        "Storage over limit ({} GB), pruning oldest recordings",
        cfg.storage.max_gb
    );

    let mut first = true;
    loop {
        if !storage::is_over_limit(&cfg.recording.output_dir, cfg.storage.max_gb) {
            break;
        }

        let deleted = tokio::task::spawn_blocking(|| -> anyhow::Result<bool> {
            let conn = db::open()?;
            match db::get_oldest_match(&conn)? {
                Some(m) => {
                    let _ = storage::delete_video_file(&m.video_path);
                    db::delete_match(&conn, m.id)?;
                    Ok(true)
                }
                None => Ok(false),
            }
        })
        .await;

        match deleted {
            Ok(Ok(true)) => {
                if first {
                    first = false;
                    let _ = app.emit(
                        "storage-limit-exceeded",
                        serde_json::json!({ "max_gb": cfg.storage.max_gb }),
                    );
                }
            }
            _ => break,
        }
    }
}

fn rename_recording(current: &str, champion: &str, result: &str, output_dir: &str) -> String {
    let date = Local::now().format("%Y-%m-%d");
    let base = format!("{}_{}_{}.mp4", date, champion, result);
    let mut candidate = std::path::Path::new(output_dir).join(&base);

    // Avoid overwriting an existing file
    let mut n = 2u32;
    while candidate.exists() {
        candidate = std::path::Path::new(output_dir)
            .join(format!("{}_{}_{}-{}.mp4", date, champion, result, n));
        n += 1;
    }

    if let Err(e) = std::fs::rename(current, &candidate) {
        warn!("Could not rename recording: {e}");
        return current.to_string();
    }

    candidate.to_string_lossy().to_string()
}

fn uuid_fallback() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("local_{ms}")
}

fn open_in_explorer(path: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

// ─── Entry Point ──────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("match_lens_lib=debug".parse().unwrap()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::default())
        .setup(|app| {
            let handle = app.handle().clone();

            setup_tray(&handle)?;

            // Init DB
            if let Err(e) = db::open() {
                error!("Failed to open database: {e}");
            }

            let cfg = config::load();

            // Autostart registration
            info!("Autostart: {}", if cfg.app.autostart { "enabled" } else { "disabled" });
            if let Err(e) = set_autostart(cfg.app.autostart) {
                warn!("Autostart registration failed: {e}");
            }

            // Spawn main recording loop
            tauri::async_runtime::spawn(run_recording_loop(handle.clone()));

            // Spawn LCU credential + summoner refresher
            tauri::async_runtime::spawn({
                let h = handle.clone();
                async move {
                    loop {
                        let state = h.state::<AppState>();
                        match lcu::read_lockfile() {
                            Ok(creds) => {
                                *state.lcu_creds.lock().await = Some(creds.clone());

                                // Always refresh the summoner name so account switches are picked up.
                                let client =
                                    lcu::build_lcu_client().unwrap_or_else(|_| Client::new());
                                if let Ok(name) =
                                    lcu::fetch_summoner_name(&client, &creds).await
                                {
                                    *state.summoner_name.lock().await = name;
                                }
                            }
                            Err(_) => {
                                *state.lcu_creds.lock().await = None;
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_matches,
            get_events,
            get_network_samples,
            get_settings,
            save_settings,
            get_recording_status,
            delete_recording,
            open_recordings_folder,
            get_storage_usage,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
