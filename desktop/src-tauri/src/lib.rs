mod tray;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

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
        Self { entries: Vec::new(), max_entries: max }
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
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct AppConfig {
    media_path: String,
    tmdb_key: String,
    server_name: String,
    port: u16,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            media_path: String::new(),
            tmdb_key: String::new(),
            server_name: "Cast Server".to_string(),
            port: 3456,
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
        server_name: std::env::var("CAST_SERVER_NAME").unwrap_or_else(|_| "Cast Server".to_string()),
        port: std::env::var("CAST_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3456),
    }
}

/// Save config to .env file
fn save_config_to_env(config: &AppConfig) -> Result<(), String> {
    let exe_dir = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("No parent dir")?
        .to_path_buf();

    let env_path = exe_dir.join(".env");
    let mut content = format!("CAST_MEDIA_PATH={}\n", config.media_path);
    if !config.tmdb_key.is_empty() {
        content += &format!("TMDB_API_KEY={}\n", config.tmdb_key);
    }
    if config.server_name != "Cast Server" {
        content += &format!("CAST_SERVER_NAME={}\n", config.server_name);
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

    logs.lock().unwrap().push("Starting Cast server...".to_string());
    let _ = app_handle.emit("server-log", "Starting Cast server...");

    let server_config = cast_server::ServerConfig {
        media_path: PathBuf::from(&config.media_path),
        port: config.port,
        name: config.server_name.clone(),
        tmdb_key: if config.tmdb_key.is_empty() { None } else { Some(config.tmdb_key.clone()) },
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

    let desktop_state = DesktopState {
        logs: logs.clone(),
        server_running: server_running.clone(),
        config: Arc::new(Mutex::new(config.clone())),
    };

    tracing_subscriber::fmt()
        .with_env_filter("info,mdns_sd=warn")
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(desktop_state)
        .invoke_handler(tauri::generate_handler![
            get_logs,
            get_config,
            save_config,
            restart_server,
            is_server_running,
            get_server_stats,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Hide to tray instead of quitting
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .setup(move |app| {
            tray::setup_tray(app.handle())?;

            // Auto-start server if media path is configured
            if !config.media_path.is_empty() {
                let logs = logs.clone();
                let server_running = server_running.clone();
                let app_handle = app.handle().clone();

                let server_config = cast_server::ServerConfig {
                    media_path: PathBuf::from(&config.media_path),
                    port: config.port,
                    name: config.server_name.clone(),
                    tmdb_key: if config.tmdb_key.is_empty() { None } else { Some(config.tmdb_key.clone()) },
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

                    logs.lock().unwrap().push("Starting Cast server...".to_string());

                    match cast_server::start_server(server_config, Some(log_cb)).await {
                        Ok(_handle) => {
                            *server_running.lock().unwrap() = true;
                            logs.lock().unwrap().push("Server is running".to_string());
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
