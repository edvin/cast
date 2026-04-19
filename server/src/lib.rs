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
pub type BoxedLogCallback = Box<dyn Fn(&str) + Send + Sync>;
/// One entry per series that needs TMDB metadata fetching.
/// Fields: (series_id, folder_name, has_art, has_backdrop, tmdb_id_override)
pub type TmdbFetchEntry = (String, String, bool, bool, Option<u64>);

pub struct AppState {
    pub library: RwLock<library::Library>,
    pub db: db::Database,
    pub media_path: PathBuf,
    pub tmdb: Option<tmdb::TmdbClient>,
    pub active_streams: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    pub remuxing: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// Episode IDs currently having their thumbnail generated — prevents duplicate
    /// ffmpeg invocations when the UI asks for the same thumbnail concurrently.
    pub generating_thumbs: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// Caps concurrent thumbnail ffmpegs so loading a 20-episode series page doesn't
    /// spawn 20 ffmpegs at once.
    pub thumb_semaphore: Arc<tokio::sync::Semaphore>,
    /// The H.264 encoder (name + ffmpeg args) to use when a file needs re-encoding.
    /// Picked at start_server based on config + probe results.
    pub transcode_encoder: (&'static str, &'static str),
    /// Human-readable label for the encoder, surfaced via /api/hwenc and the UI.
    pub encoder_label: String,
    pub log: Option<LogCallback>,
}

impl AppState {
    /// Log a message to stderr, tracing, and the UI callback.
    pub fn log(&self, msg: &str) {
        eprintln!("[cast] {msg}");
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
    /// Encoder override: "auto" (default) | nvenc | qsv | amf | videotoolbox | software
    pub encoder_override: Option<String>,
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
    log_callback: Option<BoxedLogCallback>,
) -> Result<ServerHandle, Box<dyn std::error::Error>> {
    let log_cb: Option<LogCallback> = log_callback.map(|cb| Arc::from(cb) as LogCallback);
    // Every log line goes through this closure: stderr (for CLI), tracing (for env-filter
    // consumers), and the UI callback (the desktop app's Logs tab).
    let log = |msg: &str, cb: &Option<LogCallback>| {
        eprintln!("[cast] {msg}");
        tracing::info!("{msg}");
        if let Some(ref f) = cb {
            f(msg);
        }
    };

    // Canary: prove the callback plumbing works. If the user doesn't see this line in
    // the desktop Logs tab, the issue is the callback itself, not a later hang.
    log(
        &format!(
            "Cast server v{} starting (log callback wired)",
            env!("CARGO_PKG_VERSION")
        ),
        &log_cb,
    );

    let media_path = config.media_path.canonicalize().map_err(|e| {
        let msg = format!("Media directory does not exist: {:?} ({e})", config.media_path);
        log(&msg, &log_cb);
        msg
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

    log(&format!("Scanning media directory: {media_path:?}"), &log_cb);

    log("Checking for ffprobe...", &log_cb);
    if media::is_ffprobe_available() {
        log("ffprobe detected", &log_cb);
    } else {
        log("ffprobe NOT found in PATH (thumbnails/duration disabled)", &log_cb);
    }
    log("Checking for ffmpeg...", &log_cb);
    let ffmpeg_ok = media::is_ffmpeg_available();
    if ffmpeg_ok {
        log("ffmpeg detected", &log_cb);
        log("Probing hardware encoders (NVENC/QSV/AMF/VideoToolbox)...", &log_cb);
        for (name, _args, result) in routes::probe_all_encoders() {
            match result {
                Ok(()) => log(&format!("  {name}: OK"), &log_cb),
                Err(reason) => log(&format!("  {name}: unavailable — {reason}"), &log_cb),
            }
        }
    } else {
        log(
            "ffmpeg NOT found in PATH (playback will fail for non-MP4 files)",
            &log_cb,
        );
    }

    // Resolve encoder based on config (or CAST_ENCODER env var if config didn't set it).
    let encoder_choice = config
        .encoder_override
        .clone()
        .or_else(|| std::env::var("CAST_ENCODER").ok());
    let (transcode_encoder, encoder_msg) = routes::resolve_encoder(encoder_choice.as_deref());
    if ffmpeg_ok {
        log(&encoder_msg, &log_cb);
    }
    let encoder_label = routes::label_for(transcode_encoder.0);

    log("Opening cast.db...", &log_cb);
    let db = db::Database::new(&media_path).map_err(|e| {
        let msg = format!("Failed to open cast.db: {e}");
        log(&msg, &log_cb);
        msg
    })?;
    log("cast.db ready", &log_cb);

    log("Scanning library on disk...", &log_cb);
    let lib = library::Library::scan(&media_path).map_err(|e| {
        let msg = format!("Library scan failed: {e}");
        log(&msg, &log_cb);
        msg
    })?;

    log(
        &format!(
            "Found {} series with {} episodes",
            lib.series.len(),
            lib.series.values().map(|s| s.episodes.len()).sum::<usize>()
        ),
        &log_cb,
    );

    let tmdb_client = config.tmdb_key.map(|key| {
        log("TMDB integration enabled", &log_cb);
        tmdb::TmdbClient::new(key)
    });

    // Collect the initial list of series that need TMDB fetching. We don't block startup
    // on the fetch itself — the HTTP listener and background remux start immediately and
    // the fetch runs in a spawned task so the Logs UI sees per-series progress in parallel.
    let initial_tmdb_work: Option<Vec<TmdbFetchEntry>> = tmdb_client.as_ref().map(|_| {
        lib.series
            .values()
            .filter(|s| s.art.is_none() || s.backdrop.is_none() || db.get_series_metadata(&s.id).is_none())
            .map(|s| {
                (
                    s.id.clone(),
                    s.title.clone(),
                    s.art.is_some(),
                    s.backdrop.is_some(),
                    s.tmdb_id_override,
                )
            })
            .collect()
    });

    let state = Arc::new(AppState {
        library: RwLock::new(lib),
        db,
        media_path: media_path.clone(),
        tmdb: tmdb_client,
        active_streams: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        remuxing: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        generating_thumbs: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        thumb_semaphore: Arc::new(tokio::sync::Semaphore::new(2)),
        transcode_encoder,
        encoder_label,
        log: log_cb,
    });

    // Kick off the initial TMDB fetch in the background (if needed). Progress lines flow
    // to the UI via state.log. After the fetch completes the library is rescanned so the
    // newly downloaded art files are picked up.
    if let Some(work) = initial_tmdb_work {
        if !work.is_empty() && state.tmdb.is_some() {
            let tmdb_state = state.clone();
            let tmdb_path = media_path.clone();
            tokio::spawn(async move {
                let client = match tmdb_state.tmdb.as_ref() {
                    Some(c) => c,
                    None => return,
                };
                let count = work.len();
                tmdb_state.log(&format!("Fetching TMDB metadata for {count} series..."));
                let log_state = tmdb_state.clone();
                let downloaded =
                    tmdb::fetch_all_metadata(client, &tmdb_state.db, &tmdb_path, work, move |msg| log_state.log(msg))
                        .await;
                if downloaded > 0 {
                    tmdb_state.log(&format!("Downloaded artwork for {downloaded} series"));
                }
                tmdb_state.log("TMDB metadata fetch complete");
                if let Ok(updated) = library::Library::scan(&tmdb_path) {
                    *tmdb_state.library.write().await = updated;
                }
            });
        }
    }

    // mDNS advertisement
    let mdns_name = config.name.clone();
    let mdns_port = config.port;
    let mdns_state = state.clone();
    tokio::spawn(async move {
        let log_state = mdns_state.clone();
        if let Err(e) = mdns::advertise(&mdns_name, mdns_port, move |msg| log_state.log(msg)).await {
            mdns_state.log(&format!("mDNS advertisement failed: {e}"));
        }
    });

    // Periodic library rescan + TMDB metadata fetch for new series
    let rescan_state = state.clone();
    let rescan_path = media_path.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        let mut prev_series_count = rescan_state.library.read().await.series.len();
        let mut prev_episode_count: usize = rescan_state
            .library
            .read()
            .await
            .series
            .values()
            .map(|s| s.episodes.len())
            .sum();

        loop {
            interval.tick().await;
            match library::Library::scan(&rescan_path) {
                Ok(lib) => {
                    let new_series = lib.series.len();
                    let new_episodes: usize = lib.series.values().map(|s| s.episodes.len()).sum();

                    // Log if something changed
                    if new_series != prev_series_count || new_episodes != prev_episode_count {
                        rescan_state.log(&format!(
                            "Library updated: {new_series} series, {new_episodes} episodes (was {prev_series_count}, {prev_episode_count})"
                        ));
                        prev_series_count = new_series;
                        prev_episode_count = new_episodes;
                    }

                    // Fetch TMDB metadata for series that don't have it yet
                    if let Some(ref client) = rescan_state.tmdb {
                        let series_needing_metadata: Vec<_> = lib
                            .series
                            .values()
                            .filter(|s| {
                                s.art.is_none()
                                    || s.backdrop.is_none()
                                    || rescan_state.db.get_series_metadata(&s.id).is_none()
                            })
                            .map(|s| {
                                (
                                    s.id.clone(),
                                    s.title.clone(),
                                    s.art.is_some(),
                                    s.backdrop.is_some(),
                                    s.tmdb_id_override,
                                )
                            })
                            .collect();

                        if !series_needing_metadata.is_empty() {
                            let count = series_needing_metadata.len();
                            rescan_state.log(&format!("Fetching TMDB metadata for {count} series..."));
                            let log_state = rescan_state.clone();
                            let downloaded = tmdb::fetch_all_metadata(
                                client,
                                &rescan_state.db,
                                &rescan_path,
                                series_needing_metadata,
                                move |msg| log_state.log(msg),
                            )
                            .await;
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
                Err(e) => rescan_state.log(&format!("Rescan failed: {e}")),
            }
        }
    });

    // Background pre-remux
    let remux_state = state.clone();
    let remux_path = media_path.clone();
    tokio::spawn(async move {
        // Short grace period so startup log lines can land first, then begin work immediately.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        remux_state.log("Background remux task started");
        const MAX_REMUX_ATTEMPTS: i64 = 3;
        loop {
            let files_to_remux: Vec<(PathBuf, PathBuf, String, String, String, String)> = {
                let lib = remux_state.library.read().await;
                let mut files = Vec::new();
                let mut skipped_abandoned: u32 = 0;
                for series in lib.series.values() {
                    for ep in &series.episodes {
                        let ep_path = remux_path.join(&ep.path);
                        if !routes::needs_remux(&ep_path) {
                            continue;
                        }
                        let stem = ep_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                        let mp4_path = ep_path.parent().unwrap().join(format!("{stem}.mp4"));
                        let tmp_path = ep_path.parent().unwrap().join(format!("{stem}.mp4.tmp"));
                        if mp4_path.exists() || tmp_path.exists() {
                            continue;
                        }
                        if remux_state.db.is_remux_abandoned(&ep.id) {
                            skipped_abandoned += 1;
                            continue;
                        }
                        files.push((
                            ep_path,
                            mp4_path,
                            stem,
                            ep.id.clone(),
                            series.id.clone(),
                            ep.path.clone(),
                        ));
                    }
                }
                if skipped_abandoned > 0 {
                    remux_state.log(&format!(
                        "Skipping {skipped_abandoned} abandoned file(s) from previous failed attempts — use Retry to try again"
                    ));
                }
                files
            };

            if files_to_remux.is_empty() {
                remux_state.log("All files are Apple TV ready");
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                continue;
            }
            let total_files = files_to_remux.len();
            remux_state.log(&format!("{total_files} files need conversion"));

            for (index, (source, target, stem, episode_id, series_id, ep_rel_path)) in files_to_remux.iter().enumerate()
            {
                if target.exists() {
                    continue;
                }

                // Skip if another path (on-demand, streaming, batch) is already remuxing this file
                {
                    let mut set = remux_state.remuxing.lock().unwrap();
                    if set.contains(stem) {
                        continue;
                    }
                    set.insert(stem.clone());
                }

                let tmp_path = target.parent().unwrap().join(format!("{stem}.mp4.tmp"));
                let (video_codec, video_extra) = routes::detect_video_codec(source, remux_state.transcode_encoder);
                let action = if video_codec == "copy" {
                    "Remuxing"
                } else {
                    "Transcoding"
                };
                remux_state.log(&format!(
                    "{action} [{}/{}]: {}",
                    index + 1,
                    total_files,
                    source.file_name().unwrap().to_string_lossy()
                ));

                // Heartbeat on a dedicated OS thread so it keeps ticking even if the async
                // runtime is busy (TMDB fetches, IPC, etc.). Reports wall-clock elapsed.
                let source_size = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
                let hb_tmp = tmp_path.clone();
                let hb_state = remux_state.clone();
                let hb_label = format!(
                    "{action} [{}/{}]: {}",
                    index + 1,
                    total_files,
                    source.file_name().unwrap().to_string_lossy()
                );
                let hb_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let hb_stop_flag = hb_stop.clone();
                let heartbeat = std::thread::spawn(move || {
                    let started = std::time::Instant::now();
                    // Poll the stop flag in small increments so abort is responsive, but
                    // only log every 30s.
                    let mut next_report = std::time::Duration::from_secs(30);
                    while !hb_stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        if started.elapsed() < next_report {
                            continue;
                        }
                        next_report += std::time::Duration::from_secs(30);
                        let elapsed_secs = started.elapsed().as_secs();
                        let tmp_size = std::fs::metadata(&hb_tmp).map(|m| m.len()).unwrap_or(0);
                        if source_size > 0 {
                            let pct = ((tmp_size as f64 / source_size as f64) * 100.0).min(99.0) as u32;
                            hb_state.log(&format!("  {hb_label} — {pct}% ({elapsed_secs}s elapsed)"));
                        } else {
                            hb_state.log(&format!("  {hb_label} — {elapsed_secs}s elapsed"));
                        }
                    }
                });

                let source_clone = source.clone();
                let tmp_clone = tmp_path.clone();
                let vc = video_codec.to_string();
                let ve = video_extra.to_string();
                let result = tokio::task::spawn_blocking(move || {
                    let video_codec = vc.as_str();
                    let video_extra = ve.as_str();
                    let mut cmd = media::ffmpeg_command();
                    cmd.arg("-hide_banner")
                        .arg("-loglevel")
                        .arg("warning")
                        .arg("-i")
                        .arg(&source_clone)
                        .arg("-c:v")
                        .arg(video_codec);
                    if video_codec != "copy" {
                        for part in video_extra.split_whitespace() {
                            cmd.arg(part);
                        }
                    }
                    cmd.arg("-c:a")
                        .arg("aac")
                        .arg("-b:a")
                        .arg("192k")
                        .arg("-ac")
                        .arg("2")
                        .arg("-map")
                        .arg("0:v:0")
                        .arg("-map")
                        .arg("0:a:0")
                        .arg("-map")
                        .arg("0:s?")
                        .arg("-c:s")
                        .arg("mov_text")
                        .arg("-movflags")
                        .arg("+faststart")
                        .arg("-f")
                        .arg("mp4")
                        .arg("-y")
                        .arg(&tmp_clone)
                        .output()
                })
                .await;
                hb_stop.store(true, std::sync::atomic::Ordering::Relaxed);
                let _ = heartbeat.join();

                match result {
                    Ok(Ok(output)) if output.status.success() => {
                        if std::fs::rename(&tmp_path, target).is_ok() {
                            remux_state.log(&format!(
                                "{action} complete [{}/{}]: {}",
                                index + 1,
                                total_files,
                                target.file_name().unwrap().to_string_lossy()
                            ));
                            remux_state.db.clear_remux_failure(episode_id);
                            let is_streaming = remux_state
                                .active_streams
                                .lock()
                                .map(|s| s.contains(stem))
                                .unwrap_or(false);
                            if !is_streaming && std::fs::remove_file(source).is_ok() {
                                remux_state.log(&format!(
                                    "Deleted original: {}",
                                    source.file_name().unwrap().to_string_lossy()
                                ));
                            }
                        }
                    }
                    Ok(Ok(output)) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let first_err = stderr.lines().next().unwrap_or("unknown error").to_string();
                        remux_state.log(&format!("{action} failed: {first_err}"));
                        let _ = std::fs::remove_file(&tmp_path);
                        let state_after = remux_state.db.record_remux_failure(
                            episode_id,
                            series_id,
                            ep_rel_path,
                            &first_err,
                            MAX_REMUX_ATTEMPTS,
                        );
                        if state_after.given_up {
                            remux_state.log(&format!(
                                "Giving up on {} after {} attempts — use Retry to try again",
                                source.file_name().unwrap().to_string_lossy(),
                                state_after.attempts
                            ));
                        }
                    }
                    _ => {
                        if let Ok(mut set) = remux_state.remuxing.lock() {
                            set.remove(stem);
                        }
                        break;
                    }
                }
                if let Ok(mut set) = remux_state.remuxing.lock() {
                    set.remove(stem);
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
        if !remux_dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&remux_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("mp4") {
                    if let Some(stem) = path.file_stem() {
                        let series_dir = media_path.join(&series.title);
                        if !known_stems.contains(&(
                            series_dir.to_string_lossy().to_string(),
                            stem.to_string_lossy().to_string(),
                        )) {
                            tracing::info!("Removing orphaned remux: {:?}", path.file_name().unwrap());
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }
            if std::fs::read_dir(&remux_dir)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
            {
                let _ = std::fs::remove_dir(&remux_dir);
            }
        }
    }
}
