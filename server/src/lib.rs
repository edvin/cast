pub mod db;
pub mod library;
pub mod mdns;
pub mod media;
pub mod routes;
pub mod subtitle;
pub mod tmdb;

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct AppState {
    pub library: RwLock<library::Library>,
    pub db: db::Database,
    pub media_path: PathBuf,
    pub tmdb: Option<tmdb::TmdbClient>,
    pub active_streams: std::sync::Mutex<std::collections::HashSet<String>>,
}

/// Configuration for starting the Cast server
pub struct ServerConfig {
    pub media_path: PathBuf,
    pub port: u16,
    pub name: String,
    pub tmdb_key: Option<String>,
}

/// A handle to the running server, can be used to get state info
pub struct ServerHandle {
    pub state: Arc<AppState>,
    pub port: u16,
    pub name: String,
}

/// Start the Cast server. Returns a handle for querying state.
/// The server runs until the future is dropped or the process exits.
pub async fn start_server(
    config: ServerConfig,
    log_callback: Option<Box<dyn Fn(&str) + Send + Sync>>,
) -> Result<ServerHandle, Box<dyn std::error::Error>> {
    let media_path = config.media_path.canonicalize().map_err(|_| {
        format!("Media directory does not exist: {:?}", config.media_path)
    })?;

    // On Windows, canonicalize() returns \\?\ extended-length paths which break ffmpeg.
    // Strip the prefix to get a normal path.
    #[cfg(target_os = "windows")]
    let media_path = {
        let s = media_path.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            std::path::PathBuf::from(stripped)
        } else {
            media_path
        }
    };

    let msg = format!("Scanning media directory: {:?}", media_path);
    tracing::info!("{msg}");
    if let Some(ref cb) = log_callback { cb(&msg); }

    if media::is_ffprobe_available() {
        tracing::info!("ffprobe detected");
    }
    if media::is_ffmpeg_available() {
        tracing::info!("ffmpeg detected");
    }

    let db = db::Database::new(&media_path)?;
    let lib = library::Library::scan(&media_path)?;

    let msg = format!(
        "Found {} series with {} episodes",
        lib.series.len(),
        lib.series.values().map(|s| s.episodes.len()).sum::<usize>()
    );
    tracing::info!("{msg}");
    if let Some(ref cb) = log_callback { cb(&msg); }

    let tmdb_client = config.tmdb_key.map(|key| {
        tracing::info!("TMDB integration enabled");
        tmdb::TmdbClient::new(key)
    });

    // Fetch metadata on startup if needed
    if let Some(ref client) = tmdb_client {
        let series_info: Vec<(String, String, bool, Option<u64>)> = lib
            .series.values()
            .map(|s| (s.id.clone(), s.title.clone(), s.art.is_some(), s.tmdb_id_override))
            .collect();

        let needs_fetch = series_info.iter()
            .any(|(id, _, has_art, _)| !has_art || db.get_series_metadata(id).is_none());

        if needs_fetch {
            let msg = "Fetching metadata from TMDB...";
            tracing::info!("{msg}");
            if let Some(ref cb) = log_callback { cb(msg); }
            let downloaded = tmdb::fetch_all_metadata(client, &db, &media_path, series_info).await;
            let msg = format!("Downloaded artwork for {downloaded} series");
            tracing::info!("{msg}");
            if let Some(ref cb) = log_callback { cb(&msg); }
        }
    }

    let lib = library::Library::scan(&media_path)?;

    let state = Arc::new(AppState {
        library: RwLock::new(lib),
        db,
        media_path: media_path.clone(),
        tmdb: tmdb_client,
        active_streams: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    // mDNS advertisement
    let mdns_name = config.name.clone();
    let mdns_port = config.port;
    tokio::spawn(async move {
        if let Err(e) = mdns::advertise(&mdns_name, mdns_port).await {
            tracing::warn!("mDNS advertisement failed: {e}");
        }
    });

    // Periodic library rescan
    let rescan_state = state.clone();
    let rescan_path = media_path.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            match library::Library::scan(&rescan_path) {
                Ok(lib) => *rescan_state.library.write().await = lib,
                Err(e) => tracing::warn!("Rescan failed: {e}"),
            }
        }
    });

    // Background pre-remux
    let remux_state = state.clone();
    let remux_path = media_path.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        loop {
            let files_to_remux = {
                let lib = remux_state.library.read().await;
                let mut files = Vec::new();
                for series in lib.series.values() {
                    for ep in &series.episodes {
                        let ep_path = remux_path.join(&ep.path);
                        if routes::needs_remux(&ep_path) {
                            let stem = ep_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                            let mp4_path = ep_path.parent().unwrap().join(format!("{stem}.mp4"));
                            if !mp4_path.exists() {
                                files.push((ep_path, mp4_path, stem));
                            }
                        }
                    }
                }
                files
            };

            if files_to_remux.is_empty() {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                continue;
            }

            for (source, target, stem) in &files_to_remux {
                if target.exists() { continue; }

                let tmp_path = target.parent().unwrap().join(format!("{stem}.mp4.tmp"));
                tracing::info!("Background remux: {:?}", source.file_name().unwrap());

                let source_clone = source.clone();
                let tmp_clone = tmp_path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let (video_codec, video_extra) = routes::detect_video_codec(&source_clone);
                    let mut cmd = std::process::Command::new("ffmpeg");
                    cmd.arg("-hide_banner").arg("-loglevel").arg("warning")
                        .arg("-i").arg(&source_clone).arg("-c:v").arg(video_codec);
                    if video_codec != "copy" {
                        for part in video_extra.split_whitespace() { cmd.arg(part); }
                    }
                    cmd.arg("-c:a").arg("aac").arg("-b:a").arg("192k").arg("-ac").arg("2")
                        .arg("-map").arg("0:v:0").arg("-map").arg("0:a:0")
                        .arg("-map").arg("0:s?").arg("-c:s").arg("mov_text")
                        .arg("-movflags").arg("+faststart").arg("-y").arg(&tmp_clone)
                        .output()
                }).await;

                match result {
                    Ok(Ok(output)) if output.status.success() => {
                        if std::fs::rename(&tmp_path, target).is_ok() {
                            tracing::info!("Background remux complete: {:?}", target.file_name().unwrap());
                            let is_streaming = remux_state.active_streams.lock()
                                .map(|s| s.contains(stem)).unwrap_or(false);
                            if !is_streaming {
                                if std::fs::remove_file(source).is_ok() {
                                    tracing::info!("Deleted original: {:?}", source.file_name().unwrap());
                                }
                            }
                        }
                    }
                    Ok(Ok(output)) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        tracing::warn!("Background remux failed: {stderr}");
                        let _ = std::fs::remove_file(&tmp_path);
                    }
                    _ => break,
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }

            if let Ok(lib) = library::Library::scan(&remux_path) {
                *remux_state.library.write().await = lib;
            }
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        }
    });

    // Hourly cleanup
    let cleanup_state = state.clone();
    let cleanup_path = media_path.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let lib = cleanup_state.library.read().await;
            cleanup_remux_cache(&cleanup_path, &lib);
        }
    });

    // Initial cleanup
    {
        let lib = state.library.read().await;
        cleanup_remux_cache(&media_path, &lib);
    }

    let app = routes::create_router(state.clone());

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
    let msg = format!("Cast server listening on 0.0.0.0:{}", config.port);
    tracing::info!("{msg}");
    if let Some(ref cb) = log_callback { cb(&msg); }

    let handle = ServerHandle {
        state,
        port: config.port,
        name: config.name,
    };

    // Serve in background
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    Ok(handle)
}

fn cleanup_remux_cache(media_path: &std::path::Path, lib: &library::Library) {
    let mut known_stems: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for series in lib.series.values() {
        for ep in &series.episodes {
            let ep_path = media_path.join(&ep.path);
            if let (Some(parent), Some(stem)) = (ep_path.parent(), ep_path.file_stem()) {
                known_stems.insert((parent.to_string_lossy().to_string(), stem.to_string_lossy().to_string()));
            }
        }
    }
    for series in lib.series.values() {
        let remux_dir = media_path.join(&series.title).join(".remux");
        if !remux_dir.is_dir() { continue; }
        if let Ok(entries) = std::fs::read_dir(&remux_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("mp4") {
                    if let Some(stem) = path.file_stem() {
                        let series_dir = media_path.join(&series.title);
                        if !known_stems.contains(&(series_dir.to_string_lossy().to_string(), stem.to_string_lossy().to_string())) {
                            tracing::info!("Removing orphaned remux: {:?}", path.file_name().unwrap());
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }
            if std::fs::read_dir(&remux_dir).map(|mut d| d.next().is_none()).unwrap_or(false) {
                let _ = std::fs::remove_dir(&remux_dir);
            }
        }
    }
}
