mod db;
mod library;
mod mdns;
mod media;
mod routes;
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
    #[arg(short, long)]
    media: PathBuf,

    /// Port to listen on
    #[arg(short, long, default_value = "3456")]
    port: u16,

    /// Server display name for Bonjour
    #[arg(short = 'n', long, default_value = "Cast Server")]
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
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Set up logging — file or stdout
    let _guard = if args.log_file {
        let log_dir = args.media.join("logs");
        std::fs::create_dir_all(&log_dir).ok();
        let file_appender = tracing_appender::rolling::daily(&log_dir, "cast-server.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
            .with(tracing_subscriber::EnvFilter::new("info"))
            .init();
        Some(guard)
    } else {
        tracing_subscriber::fmt::init();
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
        let series_info: Vec<(String, String, bool)> = lib
            .series
            .values()
            .map(|s| (s.id.clone(), s.title.clone(), s.art.is_some()))
            .collect();

        let needs_fetch = series_info
            .iter()
            .any(|(id, _, has_art)| !has_art || db.get_series_metadata(id).is_none());

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

    let app = routes::create_router(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", args.port))
        .await
        .expect("Failed to bind");
    tracing::info!("Cast server listening on 0.0.0.0:{}", args.port);

    axum::serve(listener, app).await.expect("Server error");
}
