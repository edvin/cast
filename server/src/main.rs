mod db;
mod library;
mod mdns;
mod media;
mod routes;
mod subtitle;
mod tmdb;

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Parser)]
#[command(name = "cast-server", about = "Cast — local network video server")]
struct Args {
    /// Path to the media directory (series as subfolders)
    #[arg(short, long, env = "CAST_MEDIA_PATH")]
    media: PathBuf,

    /// Port to listen on
    #[arg(short, long, default_value = "3456")]
    port: u16,

    /// Server display name for Bonjour [env: CAST_SERVER_NAME]
    #[arg(short = 'n', long, default_value = "Cast Server", env = "CAST_SERVER_NAME", hide_default_value = true)]
    name: String,

    /// TMDB API key for fetching series metadata and artwork
    #[arg(long, env = "TMDB_API_KEY")]
    tmdb_key: Option<String>,

    /// Log to file in the media directory instead of stdout
    #[arg(long)]
    log_file: bool,
}

pub struct AppState {
    pub library: RwLock<library::Library>,
    pub db: db::Database,
    pub media_path: PathBuf,
    pub tmdb: Option<tmdb::TmdbClient>,
    /// Tracks file stems currently being streamed (prevents MKV deletion during playback)
    pub active_streams: std::sync::Mutex<std::collections::HashSet<String>>,
}

#[tokio::main]
async fn main() {
    // Load .env file if present (before clap parses, so env vars are available)
    // Try current directory first, then next to the executable
    match dotenvy::dotenv() {
        Ok(path) => eprintln!("Loaded .env from {}", path.display()),
        Err(e) => {
            eprintln!(".env not found in cwd ({e}), trying exe dir...");
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    let env_path = dir.join(".env");
                    match dotenvy::from_path(&env_path) {
                        Ok(()) => eprintln!("Loaded .env from {}", env_path.display()),
                        Err(e) => eprintln!(".env not found at {} either: {}", env_path.display(), e),
                    }
                }
            }
        }
    }

    let args = Args::parse();

    // Set up logging — file or stdout
    let _guard = if args.log_file {
        let log_dir = args.media.join("logs");
        std::fs::create_dir_all(&log_dir).ok();
        let file_appender = tracing_appender::rolling::daily(&log_dir, "cast-server.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
            .with(tracing_subscriber::EnvFilter::new("info,mdns_sd=warn"))
            .init();
        Some(guard)
    } else {
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(tracing_subscriber::EnvFilter::new("info,mdns_sd=warn"))
            .init();
        None
    };

    let media_path = args.media.canonicalize().unwrap_or_else(|_| {
        eprintln!("Media directory does not exist: {:?}", args.media);
        std::process::exit(1);
    });

    tracing::info!("Scanning media directory: {:?}", media_path);

    // Check for ffprobe/ffmpeg availability
    if media::is_ffprobe_available() {
        tracing::info!("ffprobe detected — episode duration probing enabled");
    } else {
        tracing::warn!("ffprobe not found — episode durations will not be available");
    }
    if media::is_ffmpeg_available() {
        tracing::info!("ffmpeg detected — thumbnail generation enabled");
    } else {
        tracing::warn!("ffmpeg not found — thumbnails will not be generated");
    }

    let db = db::Database::new(&media_path).expect("Failed to open database");
    let lib = library::Library::scan(&media_path).expect("Failed to scan library");

    tracing::info!(
        "Found {} series with {} total episodes",
        lib.series.len(),
        lib.series.values().map(|s| s.episodes.len()).sum::<usize>()
    );

    let tmdb_client = args.tmdb_key.map(|key| {
        tracing::info!("TMDB integration enabled");
        tmdb::TmdbClient::new(key)
    });

    // On startup, fetch metadata if TMDB is configured
    if let Some(ref client) = tmdb_client {
        let series_info: Vec<(String, String, bool, Option<u64>)> = lib
            .series
            .values()
            .map(|s| (s.id.clone(), s.title.clone(), s.art.is_some(), s.tmdb_id_override))
            .collect();

        let needs_fetch = series_info
            .iter()
            .any(|(id, _, has_art, _)| !has_art || db.get_series_metadata(id).is_none());

        if needs_fetch {
            tracing::info!("Fetching metadata from TMDB...");
            let downloaded = tmdb::fetch_all_metadata(client, &db, &media_path, series_info).await;
            tracing::info!("Downloaded artwork for {downloaded} series");
        }
    }

    // Rescan after art download so new posters are picked up
    let lib = library::Library::scan(&media_path).expect("Failed to rescan library");

    let state = Arc::new(AppState {
        library: RwLock::new(lib),
        db,
        media_path: media_path.clone(),
        tmdb: tmdb_client,
        active_streams: std::sync::Mutex::new(std::collections::HashSet::new()),
    });

    // Start mDNS advertisement
    let mdns_name = args.name.clone();
    let mdns_port = args.port;
    tokio::spawn(async move {
        if let Err(e) = mdns::advertise(&mdns_name, mdns_port).await {
            tracing::warn!("mDNS advertisement failed: {}", e);
        }
    });

    // Rescan library periodically
    let rescan_state = state.clone();
    let rescan_path = media_path.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            match library::Library::scan(&rescan_path) {
                Ok(lib) => *rescan_state.library.write().await = lib,
                Err(e) => tracing::warn!("Rescan failed: {}", e),
            }
        }
    });

    // Background pre-remux: convert MKV/AVI/etc to MP4 proactively
    let remux_state = state.clone();
    let remux_path = media_path.clone();
    tokio::spawn(async move {
        // Wait a bit for initial scan to settle
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
                // Nothing to do, check again in 5 minutes
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                continue;
            }

            for (source, target, stem) in &files_to_remux {
                // Skip if already done (might have been remuxed by a stream request)
                if target.exists() {
                    continue;
                }

                let tmp_path = target.parent().unwrap().join(format!("{stem}.mp4.tmp"));
                tracing::info!("Background remux: {:?}", source.file_name().unwrap());

                let (video_codec, video_extra) = routes::detect_video_codec(source);

                let mut cmd = std::process::Command::new("ffmpeg");
                cmd.arg("-hide_banner")
                    .arg("-loglevel").arg("warning")
                    .arg("-i").arg(source)
                    .arg("-c:v").arg(video_codec);

                if video_codec != "copy" {
                    for part in video_extra.split_whitespace() {
                        cmd.arg(part);
                    }
                }

                let output = cmd
                    .arg("-c:a").arg("aac")
                    .arg("-b:a").arg("192k")
                    .arg("-ac").arg("2")
                    .arg("-map").arg("0:v:0")
                    .arg("-map").arg("0:a:0")
                    .arg("-map").arg("0:s?")
                    .arg("-c:s").arg("mov_text")
                    .arg("-movflags").arg("+faststart")
                    .arg("-y")
                    .arg(&tmp_path)
                    .output();

                match output {
                    Ok(result) if result.status.success() => {
                        // Rename tmp → final
                        if std::fs::rename(&tmp_path, target).is_ok() {
                            tracing::info!("Background remux complete: {:?}", target.file_name().unwrap());

                            // Delete original if not currently being streamed
                            let is_streaming = remux_state.active_streams.lock()
                                .map(|s| s.contains(stem))
                                .unwrap_or(false);
                            if !is_streaming {
                                if std::fs::remove_file(source).is_ok() {
                                    tracing::info!("Deleted original: {:?}", source.file_name().unwrap());
                                }
                            } else {
                                tracing::info!("Skipping delete of {:?} (currently streaming)", source.file_name().unwrap());
                            }
                        }
                    }
                    Ok(result) => {
                        let stderr = String::from_utf8_lossy(&result.stderr);
                        tracing::warn!("Background remux failed for {:?}: {stderr}", source.file_name().unwrap());
                        let _ = std::fs::remove_file(&tmp_path);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to run ffmpeg for background remux: {e}");
                        break; // ffmpeg not available, stop trying
                    }
                }

                // Small pause between files to avoid overloading the system
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }

            // Rescan after remuxing to pick up new .mp4 files
            if let Ok(lib) = library::Library::scan(&remux_path) {
                *remux_state.library.write().await = lib;
            }

            // Wait before checking again
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        }
    });

    // Clean orphaned remux cache hourly
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

    let app = routes::create_router(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", args.port))
        .await
        .expect("Failed to bind");
    tracing::info!("Cast server listening on 0.0.0.0:{}", args.port);

    axum::serve(listener, app).await.expect("Server error");
}

/// Remove cached .remux/*.mp4 files whose source video no longer exists
fn cleanup_remux_cache(media_path: &std::path::Path, lib: &library::Library) {
    // Collect all known video stems
    let mut known_stems: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for series in lib.series.values() {
        for ep in &series.episodes {
            let ep_path = media_path.join(&ep.path);
            if let (Some(parent), Some(stem)) = (ep_path.parent(), ep_path.file_stem()) {
                let parent_str = parent.to_string_lossy().to_string();
                let stem_str = stem.to_string_lossy().to_string();
                known_stems.insert((parent_str, stem_str));
            }
        }
    }

    // Walk all .remux directories and remove orphaned files
    for series in lib.series.values() {
        let series_dir = media_path.join(&series.title);
        let remux_dir = series_dir.join(".remux");
        if !remux_dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&remux_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("mp4") {
                    if let Some(stem) = path.file_stem() {
                        let stem_str = stem.to_string_lossy().to_string();
                        let parent_str = series_dir.to_string_lossy().to_string();
                        if !known_stems.contains(&(parent_str, stem_str)) {
                            tracing::info!("Removing orphaned remux cache: {:?}", path.file_name().unwrap());
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
            }
            // Remove the .remux dir if empty
            if std::fs::read_dir(&remux_dir).map(|mut d| d.next().is_none()).unwrap_or(false) {
                let _ = std::fs::remove_dir(&remux_dir);
            }
        }
    }
}
