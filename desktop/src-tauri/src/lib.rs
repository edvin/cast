mod tray;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Emitter;

/// Log entries stored in memory for the UI
struct LogBuffer {
    entries: Vec<LogEntry>,
    max_entries: usize,
}

#[derive(Clone, serde::Serialize)]
struct LogEntry {
    timestamp: String,
    message: String,
}

impl LogBuffer {
    fn new(max: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_entries: max,
        }
    }

    fn push(&mut self, message: String) {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        self.entries.push(LogEntry { timestamp, message });
        if self.entries.len() > self.max_entries {
            self.entries.remove(0);
        }
    }
}

/// State shared between Tauri commands and the server
struct DesktopState {
    logs: Arc<Mutex<LogBuffer>>,
    server_running: Arc<Mutex<bool>>,
    config: Arc<Mutex<AppConfig>>,
    /// Keeps the system tray icon alive for the lifetime of the app.
    /// The tray is removed when its handle drops, so we stash it here.
    _tray: Arc<Mutex<Option<tauri::tray::TrayIcon>>>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct AppConfig {
    media_path: String,
    tmdb_key: String,
    server_name: String,
    port: u16,
    /// Encoder override for transcoding. "auto" (probes and picks the best HW encoder)
    /// or one of: nvenc, qsv, amf, videotoolbox, software/libx264.
    #[serde(default = "default_encoder")]
    encoder: String,
}

fn default_encoder() -> String {
    "auto".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            media_path: String::new(),
            tmdb_key: String::new(),
            server_name: "Cast Server".to_string(),
            port: 3456,
            encoder: default_encoder(),
        }
    }
}

/// Load config from .env file next to the executable
fn load_config() -> AppConfig {
    // Try .env in cwd, then next to exe
    let _ = dotenvy::dotenv();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = dotenvy::from_path(dir.join(".env"));
        }
    }

    AppConfig {
        media_path: std::env::var("CAST_MEDIA_PATH").unwrap_or_default(),
        tmdb_key: std::env::var("TMDB_API_KEY").unwrap_or_default(),
        server_name: std::env::var("CAST_SERVER_NAME")
            .unwrap_or_else(|_| "Cast Server".to_string()),
        port: std::env::var("CAST_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3456),
        encoder: std::env::var("CAST_ENCODER").unwrap_or_else(|_| default_encoder()),
    }
}

/// Quote a .env value: wrap in double quotes and escape `\`, `"` and `$` so that
/// paths containing spaces, backslashes (Windows) or shell metacharacters round-trip
/// through `dotenvy` without being split or interpolated.
fn env_quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$");
    format!("\"{escaped}\"")
}

/// Save config to .env file
fn save_config_to_env(config: &AppConfig) -> Result<(), String> {
    let exe_dir = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("No parent dir")?
        .to_path_buf();

    let env_path = exe_dir.join(".env");
    let mut content = format!("CAST_MEDIA_PATH={}\n", env_quote(&config.media_path));
    if !config.tmdb_key.is_empty() {
        content += &format!("TMDB_API_KEY={}\n", env_quote(&config.tmdb_key));
    }
    if config.server_name != "Cast Server" {
        content += &format!("CAST_SERVER_NAME={}\n", env_quote(&config.server_name));
    }
    content += &format!("CAST_PORT={}\n", config.port);
    if !config.encoder.is_empty() && config.encoder != "auto" {
        content += &format!("CAST_ENCODER={}\n", env_quote(&config.encoder));
    }
    std::fs::write(&env_path, content).map_err(|e| e.to_string())?;
    Ok(())
}

// --- Tauri Commands ---

#[tauri::command]
fn get_logs(state: tauri::State<'_, DesktopState>) -> Vec<LogEntry> {
    state.logs.lock().unwrap().entries.clone()
}

#[tauri::command]
fn get_config(state: tauri::State<'_, DesktopState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(state: tauri::State<'_, DesktopState>, config: AppConfig) -> Result<(), String> {
    if config.media_path.trim().is_empty() {
        return Err("Media path is required".to_string());
    }
    let media = std::path::Path::new(&config.media_path);
    if !media.exists() {
        return Err(format!(
            "Media folder does not exist: {}",
            config.media_path
        ));
    }
    if !media.is_dir() {
        return Err(format!(
            "Media path is not a directory: {}",
            config.media_path
        ));
    }
    if config.port == 0 {
        return Err("Port must be between 1 and 65535".to_string());
    }
    save_config_to_env(&config)?;
    *state.config.lock().unwrap() = config;
    Ok(())
}

#[tauri::command]
async fn restart_server(
    state: tauri::State<'_, DesktopState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let config = state.config.lock().unwrap().clone();
    if config.media_path.is_empty() {
        return Err("Media path is not configured".to_string());
    }

    let logs = state.logs.clone();
    let server_running = state.server_running.clone();

    logs.lock()
        .unwrap()
        .push("Starting Cast server...".to_string());
    let _ = app_handle.emit("server-log", "Starting Cast server...");

    let server_config = cast_server::ServerConfig {
        media_path: PathBuf::from(&config.media_path),
        port: config.port,
        name: config.server_name.clone(),
        tmdb_key: if config.tmdb_key.is_empty() {
            None
        } else {
            Some(config.tmdb_key.clone())
        },
        encoder_override: if config.encoder.is_empty() {
            None
        } else {
            Some(config.encoder.clone())
        },
    };

    let log_cb = {
        let logs = logs.clone();
        let app_handle = app_handle.clone();
        Box::new(move |msg: &str| {
            logs.lock().unwrap().push(msg.to_string());
            let _ = app_handle.emit("server-log", msg.to_string());
        }) as Box<dyn Fn(&str) + Send + Sync>
    };

    match cast_server::start_server(server_config, Some(log_cb)).await {
        Ok(_handle) => {
            *server_running.lock().unwrap() = true;
            logs.lock().unwrap().push("Server is running".to_string());
            let _ = app_handle.emit("server-log", "Server is running");
            let _ = app_handle.emit("server-status", true);
            Ok(())
        }
        Err(e) => {
            let msg = format!("Failed to start: {e}");
            logs.lock().unwrap().push(msg.clone());
            let _ = app_handle.emit("server-log", msg.clone());
            Err(msg)
        }
    }
}

#[tauri::command]
fn is_server_running(state: tauri::State<'_, DesktopState>) -> bool {
    *state.server_running.lock().unwrap()
}

#[tauri::command]
fn get_server_stats(state: tauri::State<'_, DesktopState>) -> ServerStats {
    let config = state.config.lock().unwrap();
    let running = *state.server_running.lock().unwrap();
    ServerStats {
        running,
        port: config.port,
        name: config.server_name.clone(),
        media_path: config.media_path.clone(),
    }
}

#[derive(serde::Serialize)]
struct IngestResult {
    success: bool,
    series_name: String,
    filename: String,
    message: String,
}

#[tauri::command]
fn ingest_file(
    state: tauri::State<'_, DesktopState>,
    file_path: String,
) -> Result<IngestResult, String> {
    let config = state.config.lock().unwrap().clone();
    if config.media_path.is_empty() {
        return Err("Media path not configured".to_string());
    }

    let source = std::path::Path::new(&file_path);
    if !source.exists() {
        return Err(format!("File not found: {file_path}"));
    }

    let filename = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid filename")?
        .to_string();

    // Parse series name from filename
    // Common patterns: "Show.Name.S01E03.stuff.mkv" or "Show Name - S01E03 - Title.mp4"
    let series_name = extract_series_name(&filename);

    // Create series folder if it doesn't exist
    let media_root = std::path::Path::new(&config.media_path);
    let series_dir = media_root.join(&series_name);
    std::fs::create_dir_all(&series_dir).map_err(|e| format!("Failed to create folder: {e}"))?;

    // Move/copy the file
    let dest = series_dir.join(&filename);
    if dest.exists() {
        return Err(format!("File already exists: {}", dest.display()));
    }

    // Try move first (fast, same filesystem), fall back to copy+delete
    if std::fs::rename(source, &dest).is_err() {
        std::fs::copy(source, &dest).map_err(|e| format!("Failed to copy file: {e}"))?;
        let _ = std::fs::remove_file(source); // best effort delete original
    }

    Ok(IngestResult {
        success: true,
        series_name: series_name.clone(),
        filename: filename.clone(),
        message: format!("Added {} to {}", filename, series_name),
    })
}

/// Extract series name from a video filename.
/// "Show.Name.S01E03.720p.WEB.x264-GROUP.mkv" → "Show Name"
/// "Show Name - S01E03 - Episode Title.mp4" → "Show Name"
fn extract_series_name(filename: &str) -> String {
    let stem = std::path::Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);

    // Replace dots and underscores with spaces
    let cleaned = stem.replace(['.', '_'], " ");

    // Manual search for SxxExx (we avoid pulling in a regex crate here)
    let lower = cleaned.to_lowercase();
    for (i, _) in lower.char_indices() {
        if i + 6 <= lower.len() {
            let slice = &lower[i..];
            if slice.starts_with('s') {
                // Check for SxxExx pattern
                let rest = &slice[1..];
                if let Some(e_pos) = rest.find('e') {
                    let season_part = &rest[..e_pos];
                    let ep_part = &rest[e_pos + 1..];
                    if season_part.len() <= 3
                        && !season_part.is_empty()
                        && season_part.chars().all(|c| c.is_ascii_digit())
                        && ep_part.len() >= 1
                        && ep_part.chars().take(3).all(|c| c.is_ascii_digit())
                    {
                        let name = cleaned[..i].trim().trim_end_matches('-').trim();
                        if !name.is_empty() {
                            return name.to_string();
                        }
                    }
                }
            }
        }
    }

    // Try "Name - " pattern (dash separator before episode info)
    if let Some(idx) = cleaned.find(" - ") {
        let name = cleaned[..idx].trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }

    // Fallback: strip common tags and use the whole thing
    let mut name = cleaned;
    for tag in [
        "720p", "1080p", "2160p", "4k", "web", "webrip", "hdtv", "bluray", "h264", "h265", "x264",
        "x265", "hevc", "aac",
    ] {
        if let Some(pos) = name.to_lowercase().find(tag) {
            name = name[..pos].to_string();
        }
    }
    name.trim().trim_end_matches('-').trim().to_string()
}

#[tauri::command]
fn ingest_files(
    state: tauri::State<'_, DesktopState>,
    file_paths: Vec<String>,
) -> Vec<IngestResult> {
    file_paths
        .iter()
        .map(|path| match ingest_file(state.clone(), path.clone()) {
            Ok(r) => r,
            Err(e) => IngestResult {
                success: false,
                series_name: String::new(),
                filename: std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path)
                    .to_string(),
                message: e,
            },
        })
        .collect()
}

#[tauri::command]
fn get_tools() -> ToolStatus {
    ToolStatus {
        ffmpeg: cast_server::media::is_ffmpeg_available(),
        ffprobe: cast_server::media::is_ffprobe_available(),
    }
}

#[derive(serde::Serialize)]
struct ToolStatus {
    ffmpeg: bool,
    ffprobe: bool,
}

#[tauri::command]
fn get_autostart(app_handle: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app_handle
        .autolaunch()
        .is_enabled()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_autostart(app_handle: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app_handle.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn open_folder(state: tauri::State<'_, DesktopState>, series_title: String) -> Result<(), String> {
    let config = state.config.lock().unwrap().clone();
    let folder = std::path::Path::new(&config.media_path).join(&series_title);
    if !folder.exists() {
        return Err(format!("Folder not found: {}", folder.display()));
    }
    open::that(&folder).map_err(|e| format!("Failed to open: {e}"))
}

#[tauri::command]
fn play_file(state: tauri::State<'_, DesktopState>, file_path: String) -> Result<(), String> {
    let config = state.config.lock().unwrap().clone();
    let full_path = std::path::Path::new(&config.media_path).join(&file_path);
    if !full_path.exists() {
        return Err(format!("File not found: {}", full_path.display()));
    }
    open::that(&full_path).map_err(|e| format!("Failed to open: {e}"))
}

#[derive(serde::Serialize)]
struct ServerStats {
    running: bool,
    port: u16,
    name: String,
    media_path: String,
}

pub fn run() {
    let config = load_config();
    let logs = Arc::new(Mutex::new(LogBuffer::new(500)));
    let server_running = Arc::new(Mutex::new(false));
    let tray_slot: Arc<Mutex<Option<tauri::tray::TrayIcon>>> = Arc::new(Mutex::new(None));

    let desktop_state = DesktopState {
        logs: logs.clone(),
        server_running: server_running.clone(),
        config: Arc::new(Mutex::new(config.clone())),
        _tray: tray_slot.clone(),
    };

    tracing_subscriber::fmt()
        .with_env_filter("info,mdns_sd=warn")
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(desktop_state)
        .invoke_handler(tauri::generate_handler![
            get_logs,
            get_config,
            save_config,
            restart_server,
            is_server_running,
            get_server_stats,
            ingest_file,
            ingest_files,
            play_file,
            open_folder,
            get_tools,
            get_autostart,
            set_autostart,
        ])
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // Minimize to dock instead of quitting (server keeps running)
                    let _ = window.minimize();
                    api.prevent_close();
                }
                tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                    let video_exts = ["mp4", "mkv", "avi", "webm", "mov", "m4v"];
                    let video_paths: Vec<String> = paths
                        .iter()
                        .filter(|p| {
                            p.extension()
                                .and_then(|e| e.to_str())
                                .map(|e| video_exts.contains(&e.to_lowercase().as_str()))
                                .unwrap_or(false)
                        })
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    if !video_paths.is_empty() {
                        let _ = window.emit("files-dropped", video_paths);
                    }
                }
                _ => {}
            }
        })
        .setup(move |app| {
            match tray::setup_tray(app.handle()) {
                Ok(icon) => {
                    *tray_slot.lock().unwrap() = Some(icon);
                    eprintln!("[Cast] Tray icon created successfully");
                }
                Err(e) => eprintln!("[Cast] ERROR creating tray icon: {e}"),
            }

            // Auto-start server if media path is configured
            if !config.media_path.is_empty() {
                let logs = logs.clone();
                let server_running = server_running.clone();
                let app_handle = app.handle().clone();

                let server_config = cast_server::ServerConfig {
                    media_path: PathBuf::from(&config.media_path),
                    port: config.port,
                    name: config.server_name.clone(),
                    tmdb_key: if config.tmdb_key.is_empty() {
                        None
                    } else {
                        Some(config.tmdb_key.clone())
                    },
                    encoder_override: if config.encoder.is_empty() {
                        None
                    } else {
                        Some(config.encoder.clone())
                    },
                };

                tauri::async_runtime::spawn(async move {
                    let log_cb = {
                        let logs = logs.clone();
                        let app_handle = app_handle.clone();
                        Box::new(move |msg: &str| {
                            logs.lock().unwrap().push(msg.to_string());
                            // Emit to frontend
                            let _ = app_handle.emit("server-log", msg.to_string());
                        }) as Box<dyn Fn(&str) + Send + Sync>
                    };

                    logs.lock()
                        .unwrap()
                        .push("Starting Cast server...".to_string());
                    let _ = app_handle.emit("server-log", "Starting Cast server...");

                    match cast_server::start_server(server_config, Some(log_cb)).await {
                        Ok(_handle) => {
                            *server_running.lock().unwrap() = true;
                            logs.lock().unwrap().push("Server is running".to_string());
                            let _ = app_handle.emit("server-log", "Server is running");
                            let _ = app_handle.emit("server-status", true);
                        }
                        Err(e) => {
                            let msg = format!("Failed to start: {e}");
                            logs.lock().unwrap().push(msg.clone());
                            let _ = app_handle.emit("server-log", msg);
                        }
                    }
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running Cast Desktop");
}
