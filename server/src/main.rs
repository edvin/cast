// Hide the console window on Windows when running as a background service
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Parser)]
#[command(name = "cast-server", about = "Cast — local network video server")]
struct Args {
    /// Path to the media directory
    #[arg(short, long, env = "CAST_MEDIA_PATH")]
    media: PathBuf,

    /// Port to listen on
    #[arg(short, long, default_value = "3456")]
    port: u16,

    /// Server display name for Bonjour [env: CAST_SERVER_NAME]
    #[arg(
        short = 'n',
        long,
        default_value = "Cast Server",
        env = "CAST_SERVER_NAME",
        hide_default_value = true
    )]
    name: String,

    /// TMDB API key for fetching series metadata and artwork
    #[arg(long, env = "TMDB_API_KEY")]
    tmdb_key: Option<String>,

    /// Override the transcoding encoder
    /// [values: auto, nvenc, qsv, amf, videotoolbox, software]
    #[arg(long, env = "CAST_ENCODER")]
    encoder: Option<String>,

    /// Log to file in the media directory instead of stdout
    #[arg(long)]
    log_file: bool,

    /// Enable verbose/debug logging (per-cycle scan chatter, probe details,
    /// per-item TMDB progress). Off by default.
    #[arg(long, env = "CAST_LOG_DEBUG")]
    debug_log: bool,
}

#[tokio::main]
async fn main() {
    // Load .env file
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

    // Set up logging
    if args.log_file {
        let log_dir = args.media.join("logs");
        std::fs::create_dir_all(&log_dir).ok();
        let file_appender = tracing_appender::rolling::daily(&log_dir, "cast-server.log");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
            .with(tracing_subscriber::EnvFilter::new("info,mdns_sd=warn"))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(tracing_subscriber::EnvFilter::new("info,mdns_sd=warn"))
            .init();
    }

    let config = cast_server::ServerConfig {
        media_path: args.media,
        port: args.port,
        name: args.name,
        tmdb_key: args.tmdb_key,
        encoder_override: args.encoder,
        debug_logging: args.debug_log,
    };

    match cast_server::start_server(config, None).await {
        Ok(_handle) => {
            // Server is running in background, wait forever
            std::future::pending::<()>().await;
        }
        Err(e) => {
            eprintln!("Failed to start server: {e}");
            std::process::exit(1);
        }
    }
}
