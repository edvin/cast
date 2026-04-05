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

pub type LogCallback = Arc<dyn Fn(&str) + Send + Sync>;

pub struct AppState {
    pub library: RwLock<library::Library>,
    pub db: db::Database,
    pub media_path: PathBuf,
    pub tmdb: Option<tmdb::TmdbClient>,
    pub active_streams: std::sync::Mutex<std::collections::HashSet<String>>,
    pub remuxing: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    pub log: Option<LogCallback>,
}

impl AppState {
    /// Log a message to both tracing and the UI callback
    pub fn log(&self, msg: &str) {
        tracing::info!("{msg}");
        if let Some(ref cb) = self.log {
            cb(msg);
        }
    }
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
    let log_cb: Option<LogCallback> = log_callback.map(|cb| Arc::from(cb) as LogCallback);
    let log = |msg: &str, cb: &Option<LogCallback>| {
        tracing::info!("{msg}");
        if let Some(ref f) = cb { f(msg); }
    };
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

    log(&format!("Scanning media directory: {:?}", media_path), &log_cb);

    if media::is_ffprobe_available() {
        log("ffprobe detected", &log_cb);
    }
    if media::is_ffmpeg_available() {
        log("ffmpeg detected", &log_cb);
    }

    let db = db::Database::new(&media_path)?;
    let lib = library::Library::scan(&media_path)?;

    log(&format!(
        "Found {} series with {} episodes",
        lib.series.len(),
        lib.series.values().map(|s| s.episodes.len()).sum::<usize>()
    ), &log_cb);

    let tmdb_client = config.tmdb_key.map(|key| {
        log("TMDB integration enabled", &log_cb);
        tmdb::TmdbClient::new(key)
    });

    if let Some(ref client) = tmdb_client {
        let series_info: Vec<(String, String, bool, bool, Option<u64>)> = lib
            .series.values()
            .map(|s| (s.id.clone(), s.title.clone(), s.art.is_some(), s.backdrop.is_some(), s.tmdb_id_override))
            .collect();

        let needs_fetch_count = series_info.iter()
            .filter(|(id, _, has_art, has_backdrop, _)| !has_art || !has_backdrop || db.get_series_metadata(id).is_none())
            .count();

        if needs_fetch_count > 0 {
            log(&format!("Fetching TMDB metadata for {needs_fetch_count} series..."), &log_cb);
            let downloaded = tmdb::fetch_all_metadata(client, &db, &media_path, series_info).await;
            if downloaded > 0 {
                log(&format!("Downloaded artwork for {downloaded} series"), &log_cb);
            }
            log("TMDB metadata fetch complete", &log_cb);
        }
    }

    let lib = library::Library::scan(&media_path)?;

    let state = Arc::new(AppState {
        library: RwLock::new(lib),
        db,
        media_path: media_path.clone(),
        tmdb: tmdb_client,
        active_streams: std::sync::Mutex::new(std::collections::HashSet::new()),
        remuxing: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        log: log_cb,
    });

    // mDNS advertisement
    let mdns_name = config.name.clone();
    let mdns_port = config.port;
    tokio::spawn(async move {
        if let Err(e) = mdns::advertise(&mdns_name, mdns_port).await {
            tracing::warn!("mDNS advertisement failed: {e}");
        }
    });

    // Periodic library rescan + TMDB metadata fetch for new series
    let rescan_state = state.clone();
    let rescan_path = media_path.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        let mut prev_series_count = rescan_state.library.read().await.series.len();
        let mut prev_episode_count: usize = rescan_state.library.read().await
            .series.values().map(|s| s.episodes.len()).sum();

        loop {
            interval.tick().await;
            match library::Library::scan(&rescan_path) {
                Ok(lib) => {
                    let new_series = lib.series.len();
                    let new_episodes: usize = lib.series.values().map(|s| s.episodes.len()).sum();

                    // Log if something changed
                    if new_series != prev_series_count || new_episodes != prev_episode_count {
                        rescan_state.log(&format!(
                            "Library updated: {} series, {} episodes (was {}, {})",
                            new_series, new_episodes, prev_series_count, prev_episode_count
                        ));
                        prev_series_count = new_series;
                        prev_episode_count = new_episodes;
                    }

                    // Fetch TMDB metadata for series that don't have it yet
                    if let Some(ref client) = rescan_state.tmdb {
                        let series_needing_metadata: Vec<_> = lib.series.values()
                            .filter(|s| {
                                !s.art.is_some() || !s.backdrop.is_some() || rescan_state.db.get_series_metadata(&s.id).is_none()
                            })
                            .map(|s| (s.id.clone(), s.title.clone(), s.art.is_some(), s.backdrop.is_some(), s.tmdb_id_override))
                            .collect();

                        if !series_needing_metadata.is_empty() {
                            let count = series_needing_metadata.len();
                            rescan_state.log(&format!("Fetching TMDB metadata for {count} series..."));
                            let downloaded = tmdb::fetch_all_metadata(
                                client, &rescan_state.db, &rescan_path, series_needing_metadata
                            ).await;
                            if downloaded > 0 {
                                rescan_state.log(&format!("Downloaded artwork for {downloaded} series"));
                            }
                            rescan_state.log("TMDB metadata fetch complete");
                            // Rescan again to pick up new art
                            if let Ok(updated_lib) = library::Library::scan(&rescan_path) {
                                *rescan_state.library.write().await = updated_lib;
                                continue;
                            }
                        }
                    }

                    *rescan_state.library.write().await = lib;
                }
                Err(e) => tracing::warn!("Rescan failed: {e}"),
            }
        }
    });

    // Background pre-remux
    let remux_state = state.clone();
    let remux_path = media_path.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        remux_state.log("Background remux task started");
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
                            let tmp_path = ep_path.parent().unwrap().join(format!("{stem}.mp4.tmp"));
                            if !mp4_path.exists() && !tmp_path.exists() {
                                files.push((ep_path, mp4_path, stem));
                            }
                        }
                    }
                }
                files
            };

            if files_to_remux.is_empty() {
                remux_state.log("All files are Apple TV ready");
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                continue;
            }
            remux_state.log(&format!("{} files need conversion", files_to_remux.len()));

            for (source, target, stem) in &files_to_remux {
                if target.exists() { continue; }

                // Skip if another path (on-demand, streaming, batch) is already remuxing this file
                {
                    let mut set = remux_state.remuxing.lock().unwrap();
                    if set.contains(stem) {
                        continue;
                    }
                    set.insert(stem.clone());
                }

                let tmp_path = target.parent().unwrap().join(format!("{stem}.mp4.tmp"));
                let (video_codec, video_extra) = routes::detect_video_codec(source);
                let action = if video_codec == "copy" { "Remuxing" } else { "Transcoding" };
                remux_state.log(&format!("{action}: {}", source.file_name().unwrap().to_string_lossy()));

                let source_clone = source.clone();
                let tmp_clone = tmp_path.clone();
                let vc = video_codec.to_string();
                let ve = video_extra.to_string();
                let result = tokio::task::spawn_blocking(move || {
                    let video_codec = vc.as_str();
                    let video_extra = ve.as_str();
                    let mut cmd = media::ffmpeg_command();
                    cmd.arg("-hide_banner").arg("-loglevel").arg("warning")
                        .arg("-i").arg(&source_clone).arg("-c:v").arg(video_codec);
                    if video_codec != "copy" {
                        for part in video_extra.split_whitespace() { cmd.arg(part); }
                    }
                    cmd.arg("-c:a").arg("aac").arg("-b:a").arg("192k").arg("-ac").arg("2")
                        .arg("-map").arg("0:v:0").arg("-map").arg("0:a:0")
                        .arg("-map").arg("0:s?").arg("-c:s").arg("mov_text")
                        .arg("-movflags").arg("+faststart").arg("-f").arg("mp4").arg("-y").arg(&tmp_clone)
                        .output()
                }).await;

                match result {
                    Ok(Ok(output)) if output.status.success() => {
                        if std::fs::rename(&tmp_path, target).is_ok() {
                            remux_state.log(&format!("{action} complete: {}", target.file_name().unwrap().to_string_lossy()));
                            let is_streaming = remux_state.active_streams.lock()
                                .map(|s| s.contains(stem)).unwrap_or(false);
                            if !is_streaming {
                                if std::fs::remove_file(source).is_ok() {
                                    remux_state.log(&format!("Deleted original: {}", source.file_name().unwrap().to_string_lossy()));
                                }
                            }
                        }
                    }
                    Ok(Ok(output)) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        remux_state.log(&format!("{action} failed: {}", stderr.lines().next().unwrap_or("unknown error")));
                        let _ = std::fs::remove_file(&tmp_path);
                    }
                    _ => {
                        if let Ok(mut set) = remux_state.remuxing.lock() { set.remove(stem); }
                        break;
                    }
                }
                if let Ok(mut set) = remux_state.remuxing.lock() { set.remove(stem); }
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
    state.log(&format!("Cast server listening on 0.0.0.0:{}", config.port));

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
