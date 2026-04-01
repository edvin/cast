mod db;
mod library;
mod mdns;
mod routes;

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

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
    #[arg(short, long, default_value = "Cast Server")]
    name: String,
}

pub struct AppState {
    pub library: RwLock<library::Library>,
    pub db: db::Database,
    pub media_path: PathBuf,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    let media_path = args.media.canonicalize().unwrap_or_else(|_| {
        eprintln!("Media directory does not exist: {:?}", args.media);
        std::process::exit(1);
    });

    tracing::info!("Scanning media directory: {:?}", media_path);

    let db = db::Database::new(&media_path).expect("Failed to open database");
    let lib = library::Library::scan(&media_path).expect("Failed to scan library");

    tracing::info!(
        "Found {} series with {} total episodes",
        lib.series.len(),
        lib.series.values().map(|s| s.episodes.len()).sum::<usize>()
    );

    let state = Arc::new(AppState {
        library: RwLock::new(lib),
        db,
        media_path: media_path.clone(),
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
