use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

// --- Error response ---

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
    code: u16,
    detail: Option<String>,
}

impl ApiError {
    fn not_found(msg: &str) -> (StatusCode, Json<ApiError>) {
        (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: msg.to_string(),
                code: 404,
                detail: None,
            }),
        )
    }

    fn forbidden(msg: &str) -> (StatusCode, Json<ApiError>) {
        (
            StatusCode::FORBIDDEN,
            Json(ApiError {
                error: msg.to_string(),
                code: 403,
                detail: None,
            }),
        )
    }

    fn internal(msg: &str) -> (StatusCode, Json<ApiError>) {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: msg.to_string(),
                code: 500,
                detail: None,
            }),
        )
    }

    fn unavailable(msg: &str) -> (StatusCode, Json<ApiError>) {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                error: msg.to_string(),
                code: 503,
                detail: Some("This feature requires a TMDB API key".to_string()),
            }),
        )
    }
}

type ApiResult<T> = Result<T, (StatusCode, Json<ApiError>)>;

pub fn create_router(state: Arc<AppState>) -> Router {
    let cors = tower_http::cors::CorsLayer::permissive();

    Router::new()
        .route("/api/series", get(list_series))
        .route("/api/series/{series_id}", get(get_series))
        .route("/api/series/{series_id}/next", get(get_next_episode))
        .route("/api/series/{series_id}/art", get(get_series_art))
        .route("/api/episodes/{episode_id}/stream", get(stream_episode))
        .route("/api/episodes/{episode_id}/progress", get(get_progress))
        .route("/api/episodes/{episode_id}/progress", post(update_progress))
        .route("/api/progress", get(get_all_progress))
        .route("/api/continue-watching", get(continue_watching))
        .route("/api/series/{series_id}/backdrop", get(get_series_backdrop))
        .route("/api/episodes/{episode_id}/thumbnail", get(get_episode_thumbnail))
        .route("/api/episodes/{episode_id}/progress", delete(delete_progress))
        .route("/api/episodes/{episode_id}/subtitles", get(list_subtitles))
        .route("/api/episodes/{episode_id}/subtitles/{language}", get(get_subtitle))
        .route("/api/series/{series_id}/progress", delete(delete_series_progress))
        .route("/api/metadata/fetch", post(fetch_metadata))
        .route("/api/episodes/{episode_id}/credits", get(get_episode_credits))
        .route("/api/person/{person_id}", get(get_person))
        .route("/api/episodes/{episode_id}/prepare", post(prepare_episode))
        .route("/api/series/{series_id}/remux", post(remux_series))
        .route("/api/rescan", post(rescan_library))
        .route("/api/series/{series_id}", delete(delete_series))
        .route("/api/episodes/watched", get(get_watched_episodes))
        .route("/api/episodes/{episode_id}", delete(delete_episode))
        .route("/api/hwenc", get(get_hwenc_info))
        .route("/api/remux/failures", get(get_remux_failures))
        .route("/api/remux/retry", post(retry_all_remux_failures))
        .route("/api/remux/retry/{episode_id}", post(retry_one_remux_failure))
        // TMDB lookup retry
        .route("/api/metadata/failures", get(get_tmdb_failures))
        .route("/api/metadata/retry", post(retry_all_tmdb_lookups))
        .route("/api/metadata/retry/{content_id}", post(retry_one_tmdb_lookup))
        // Log verbosity toggle
        .route("/api/log-level", get(get_log_level))
        .route("/api/log-level", post(set_log_level))
        // Network info (Wake-on-LAN support)
        .route("/api/network-info", get(get_network_info))
        // Movies
        .route("/api/movies", get(list_movies))
        .route("/api/movies/{movie_id}", get(get_movie))
        .route("/api/movies/{movie_id}", delete(delete_movie))
        .route("/api/movies/{movie_id}/stream", get(stream_movie))
        .route("/api/movies/{movie_id}/thumbnail", get(get_movie_thumbnail))
        .route("/api/movies/{movie_id}/art", get(get_movie_art))
        .route("/api/movies/{movie_id}/backdrop", get(get_movie_backdrop))
        .route("/api/movies/{movie_id}/progress", get(get_movie_progress))
        .route("/api/movies/{movie_id}/progress", post(update_movie_progress))
        .route("/api/movies/{movie_id}/progress", delete(delete_movie_progress))
        .route("/api/movies/{movie_id}/prepare", post(prepare_movie))
        .route("/api/movies/{movie_id}/subtitles", get(list_movie_subtitles))
        .route("/api/movies/{movie_id}/subtitles/{language}", get(get_movie_subtitle))
        .layer(cors)
        .with_state(state)
}

async fn get_remux_failures(State(state): State<Arc<AppState>>) -> Json<Vec<crate::db::RemuxFailure>> {
    Json(state.db.list_remux_failures())
}

#[derive(Serialize)]
struct RetryResponse {
    cleared: bool,
}

async fn retry_all_remux_failures(State(state): State<Arc<AppState>>) -> Json<RetryResponse> {
    state.db.retry_remux_failures(None);
    state.log("Cleared remux failure flags — files will be retried on the next scan");
    Json(RetryResponse { cleared: true })
}

async fn retry_one_remux_failure(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
) -> Json<RetryResponse> {
    state.db.retry_remux_failures(Some(&episode_id));
    state.log(&format!("Cleared remux failure for episode {episode_id}"));
    Json(RetryResponse { cleared: true })
}

// --- TMDB lookup retry ---

async fn get_tmdb_failures(State(state): State<Arc<AppState>>) -> Json<Vec<crate::db::TmdbLookupFailure>> {
    Json(state.db.list_tmdb_failures())
}

async fn retry_all_tmdb_lookups(State(state): State<Arc<AppState>>) -> Json<RetryResponse> {
    state.db.retry_tmdb_lookups();
    state.log("Cleared all TMDB lookup failure flags — items will be retried on the next scan");
    // Kick a rescan immediately so the UI sees the effect without waiting 60s
    if let Ok(lib) = crate::library::Library::scan(&state.media_path) {
        *state.library.write().await = lib;
    }
    Json(RetryResponse { cleared: true })
}

async fn retry_one_tmdb_lookup(
    State(state): State<Arc<AppState>>,
    Path(content_id): Path<String>,
) -> Json<RetryResponse> {
    state.db.clear_tmdb_failure(&content_id);
    state.log(&format!("Cleared TMDB lookup failure for {content_id}"));
    Json(RetryResponse { cleared: true })
}

#[derive(Serialize)]
struct LogLevelResponse {
    debug: bool,
}

#[derive(Deserialize)]
struct LogLevelUpdate {
    debug: bool,
}

async fn get_log_level(State(state): State<Arc<AppState>>) -> Json<LogLevelResponse> {
    Json(LogLevelResponse {
        debug: state.debug_logging.load(std::sync::atomic::Ordering::Relaxed),
    })
}

#[derive(Serialize)]
struct NetworkInterface {
    name: String,
    mac: String,
    ipv4: Option<String>,
}

#[derive(Serialize)]
struct NetworkInfo {
    hostname: String,
    /// First non-loopback MAC address found, suitable for Wake-on-LAN.
    primary_mac: Option<String>,
    interfaces: Vec<NetworkInterface>,
}

/// Return host identity details so a tvOS client can store them after a successful
/// connect and send a Wake-on-LAN magic packet later when the server is asleep.
async fn get_network_info() -> Json<NetworkInfo> {
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_default();

    // Map interface name -> MAC, then pair with the IPv4 from get_if_addrs.
    let mut interfaces: Vec<NetworkInterface> = Vec::new();
    if let Ok(addrs) = get_if_addrs::get_if_addrs() {
        for iface in addrs {
            // We only care about non-loopback IPv4 interfaces for WoL hints.
            if iface.is_loopback() {
                continue;
            }
            let ipv4 = match iface.addr {
                get_if_addrs::IfAddr::V4(ref a) => Some(a.ip.to_string()),
                _ => None,
            };
            // mac_address looks it up by interface name
            let mac = mac_address::mac_address_by_name(&iface.name)
                .ok()
                .flatten()
                .map(|m| m.to_string())
                .unwrap_or_default();
            if mac.is_empty() {
                continue;
            }
            interfaces.push(NetworkInterface {
                name: iface.name,
                mac,
                ipv4,
            });
        }
    }

    let primary_mac = interfaces
        .iter()
        .find(|i| i.ipv4.is_some())
        .map(|i| i.mac.clone())
        .or_else(|| interfaces.first().map(|i| i.mac.clone()));

    Json(NetworkInfo {
        hostname,
        primary_mac,
        interfaces,
    })
}

async fn set_log_level(State(state): State<Arc<AppState>>, Json(body): Json<LogLevelUpdate>) -> Json<LogLevelResponse> {
    state
        .debug_logging
        .store(body.debug, std::sync::atomic::Ordering::Relaxed);
    state.log(&format!(
        "Verbose logging {}",
        if body.debug { "enabled" } else { "disabled" }
    ));
    Json(LogLevelResponse { debug: body.debug })
}

#[derive(Serialize)]
struct HwEncInfo {
    encoder: &'static str,
    label: String,
    is_hardware: bool,
    hint: Option<String>,
}

/// Reports the active transcoding encoder so the desktop GUI can show a warning
/// when software-only is in use and guide the user toward a HW-capable ffmpeg build.
async fn get_hwenc_info(State(state): State<Arc<AppState>>) -> Json<HwEncInfo> {
    let enc = state.transcode_encoder.0;
    let is_hardware = enc != "libx264";
    let hint = if is_hardware {
        None
    } else if cfg!(target_os = "windows") {
        Some(
            "No GPU encoder detected. For 5-20x faster transcoding, install an ffmpeg build with \
             NVENC/QSV/AMF support — e.g. `winget install Gyan.FFmpeg` or gyan.dev 'release-full', \
             and ensure your GPU driver is up to date."
                .to_string(),
        )
    } else if cfg!(target_os = "macos") {
        Some(
            "No VideoToolbox encoder detected. Install ffmpeg via Homebrew (`brew install ffmpeg`) \
             to enable hardware transcoding."
                .to_string(),
        )
    } else {
        Some(
            "No GPU encoder detected. Install an ffmpeg build with NVENC/VAAPI support and \
             up-to-date GPU drivers for 5-20x faster transcoding."
                .to_string(),
        )
    };
    Json(HwEncInfo {
        encoder: enc,
        label: state.encoder_label.clone(),
        is_hardware,
        hint,
    })
}

// --- DTOs ---

#[derive(Serialize)]
struct SeriesListItem {
    id: String,
    title: String,
    folder_name: String,
    episode_count: usize,
    has_art: bool,
    has_backdrop: bool,
    has_metadata: bool,
    overview: Option<String>,
    genres: Option<String>,
    rating: Option<f64>,
    year: Option<String>,
    watched_count: usize,
    total_count: usize,
}

#[derive(Serialize)]
struct SeriesDetail {
    id: String,
    title: String,
    folder_name: String,
    has_art: bool,
    has_backdrop: bool,
    overview: Option<String>,
    genres: Option<String>,
    rating: Option<f64>,
    year: Option<String>,
    episodes: Vec<EpisodeItem>,
}

#[derive(Serialize)]
struct EpisodeItem {
    id: String,
    title: String,
    index: usize,
    season_number: Option<u32>,
    episode_number: Option<u32>,
    size_bytes: u64,
    duration_secs: Option<f64>,
    overview: Option<String>,
    air_date: Option<String>,
    runtime_minutes: Option<u32>,
    has_thumbnail: bool,
    still_url: Option<String>,
    subtitle_languages: Vec<String>,
    progress: Option<EpisodeProgress>,
    /// File format: "mp4", "mkv", "avi", etc.
    format: String,
    /// Video codec: "h264", "hevc", etc.
    video_codec: Option<String>,
    /// Resolution: "1080p", "720p", "4K", etc.
    resolution: Option<String>,
    /// Original filename
    filename: String,
}

#[derive(Serialize, Clone)]
struct EpisodeProgress {
    position_secs: f64,
    duration_secs: f64,
    completed: bool,
}

#[derive(Serialize)]
struct NextEpisodeResponse {
    /// The episode to play next (or resume)
    episode: Option<EpisodeItem>,
    /// Why this episode was selected
    reason: String,
}

#[derive(Serialize)]
struct ContinueWatchingItem {
    series_id: String,
    series_title: String,
    has_art: bool,
    has_backdrop: bool,
    next_episode: EpisodeItem,
    reason: String,
}

#[derive(Deserialize)]
struct ProgressUpdate {
    position_secs: f64,
    duration_secs: f64,
}

// --- Helpers ---

/// Validate that a resolved path is within the media root directory.
/// Prevents path traversal attacks even if library data is somehow corrupted.
fn safe_media_path(
    media_root: &std::path::Path,
    relative: &str,
) -> Result<std::path::PathBuf, (StatusCode, Json<ApiError>)> {
    let resolved = media_root.join(relative);
    let canonical = resolved
        .canonicalize()
        .map_err(|_| ApiError::not_found("File not found"))?;
    let canonical_root = media_root
        .canonicalize()
        .map_err(|_| ApiError::internal("Failed to resolve media path"))?;
    if !canonical.starts_with(&canonical_root) {
        tracing::warn!("Path traversal attempt blocked: {relative:?}");
        return Err(ApiError::forbidden("Access denied"));
    }
    Ok(canonical)
}

/// Build an EpisodeItem using pre-loaded maps (avoids per-episode DB queries)
fn build_episode_item_cached(
    ep: &crate::library::Episode,
    series_id: &str,
    media_path: &std::path::Path,
    progress_map: &std::collections::HashMap<String, crate::db::WatchProgress>,
    ep_meta_map: &std::collections::HashMap<(String, u32, u32), crate::db::EpisodeMetadata>,
) -> EpisodeItem {
    let progress = progress_map.get(&ep.id).map(|p| EpisodeProgress {
        position_secs: p.position_secs,
        duration_secs: p.duration_secs,
        completed: p.completed,
    });

    let tmdb_meta = ep
        .season_number
        .zip(ep.episode_number)
        .and_then(|(s, e)| ep_meta_map.get(&(series_id.to_string(), s, e)));

    let thumb_path = media_path.join(".thumbnails").join(format!("{}.jpg", ep.id));
    let has_thumbnail = thumb_path.exists();

    EpisodeItem {
        id: ep.id.clone(),
        title: tmdb_meta
            .and_then(|m| m.title.clone())
            .unwrap_or_else(|| ep.title.clone()),
        index: ep.index,
        season_number: ep.season_number,
        episode_number: ep.episode_number,
        size_bytes: ep.size_bytes,
        duration_secs: None,
        overview: tmdb_meta.and_then(|m| m.overview.clone()),
        air_date: tmdb_meta.and_then(|m| m.air_date.clone()),
        runtime_minutes: tmdb_meta.and_then(|m| m.runtime_minutes),
        has_thumbnail,
        still_url: tmdb_meta.and_then(|m| m.still_url.clone()),
        subtitle_languages: ep.subtitles.iter().map(|s| s.language.clone()).collect(),
        progress,
        format: std::path::Path::new(&ep.path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown")
            .to_lowercase(),
        video_codec: None, // populated on demand
        resolution: None,  // populated on demand
        filename: std::path::Path::new(&ep.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string(),
    }
}

/// Build an EpisodeItem with individual DB lookups (for single-episode endpoints)
fn build_episode_item(ep: &crate::library::Episode, series_id: &str, state: &AppState) -> EpisodeItem {
    let progress_map = state.db.get_all_progress_map();
    let ep_meta_map = state.db.get_all_episode_metadata();
    build_episode_item_cached(ep, series_id, &state.media_path, &progress_map, &ep_meta_map)
}

// --- Handlers ---

async fn list_series(State(state): State<Arc<AppState>>) -> Json<Vec<SeriesListItem>> {
    let lib = state.library.read().await;
    // Batch load all metadata and progress in 2 queries instead of N*2
    let all_meta = state.db.get_all_series_metadata();
    let all_progress = state.db.get_all_progress_map();

    let mut result: Vec<SeriesListItem> = lib
        .series
        .values()
        .map(|s| {
            let watched_count = s
                .episodes
                .iter()
                .filter(|ep| all_progress.get(&ep.id).map(|p| p.completed).unwrap_or(false))
                .count();
            let meta = all_meta.get(&s.id);

            SeriesListItem {
                id: s.id.clone(),
                title: meta.and_then(|m| m.title.clone()).unwrap_or_else(|| s.title.clone()),
                folder_name: s.title.clone(),
                episode_count: s.episodes.len(),
                has_art: s.art.is_some() || state.db.has_artwork(&s.id, "art"),
                has_backdrop: s.backdrop.is_some() || state.db.has_artwork(&s.id, "backdrop"),
                has_metadata: meta.is_some(),
                overview: meta.and_then(|m| m.overview.clone()),
                genres: meta.and_then(|m| m.genres.clone()),
                rating: meta.and_then(|m| m.rating),
                year: meta.and_then(|m| m.first_air_date.as_ref().map(|d| d[..4].to_string())),
                watched_count,
                total_count: s.episodes.len(),
            }
        })
        .collect();

    result.sort_by_key(|a| a.title.to_lowercase());
    Json(result)
}

async fn get_series(
    State(state): State<Arc<AppState>>,
    Path(series_id): Path<String>,
) -> ApiResult<Json<SeriesDetail>> {
    let lib = state.library.read().await;
    let series = lib
        .find_series(&series_id)
        .ok_or_else(|| ApiError::not_found("Series not found"))?;

    // 3 queries for the whole detail view
    let all_progress = state.db.get_all_progress_map();
    let all_ep_meta = state.db.get_all_episode_metadata();

    let episodes: Vec<EpisodeItem> = series
        .episodes
        .iter()
        .map(|ep| build_episode_item_cached(ep, &series.id, &state.media_path, &all_progress, &all_ep_meta))
        .collect();

    let meta = state.db.get_series_metadata(&series.id);

    Ok(Json(SeriesDetail {
        id: series.id.clone(),
        title: meta
            .as_ref()
            .and_then(|m| m.title.clone())
            .unwrap_or_else(|| series.title.clone()),
        folder_name: series.title.clone(),
        has_art: series.art.is_some(),
        has_backdrop: series.backdrop.is_some(),
        overview: meta.as_ref().and_then(|m| m.overview.clone()),
        genres: meta.as_ref().and_then(|m| m.genres.clone()),
        rating: meta.as_ref().and_then(|m| m.rating),
        year: meta
            .as_ref()
            .and_then(|m| m.first_air_date.as_ref().map(|d| d[..4].to_string())),
        episodes,
    }))
}

/// Returns the "smart next" episode for a series:
/// - If watching one (progress > 0, not completed), resume it
/// - If last watched is completed, return the next one
/// - If nothing watched, return episode 0
async fn get_next_episode(
    State(state): State<Arc<AppState>>,
    Path(series_id): Path<String>,
) -> ApiResult<Json<NextEpisodeResponse>> {
    let lib = state.library.read().await;
    let series = lib
        .find_series(&series_id)
        .ok_or_else(|| ApiError::not_found("Series not found"))?;

    if series.episodes.is_empty() {
        return Ok(Json(NextEpisodeResponse {
            episode: None,
            reason: "No episodes".to_string(),
        }));
    }

    let episode_ids: Vec<String> = series.episodes.iter().map(|e| e.id.clone()).collect();
    let all_progress = state.db.get_series_progress(&episode_ids);

    // Check if any episode is in-progress (has position but not completed)
    // Pick the most recently updated one
    let in_progress = all_progress.iter().find(|p| !p.completed && p.position_secs > 0.0);

    if let Some(current) = in_progress {
        if let Some(ep) = series.episodes.iter().find(|e| e.id == current.episode_id) {
            let mut item = build_episode_item(ep, &series.id, &state);
            item.progress = Some(EpisodeProgress {
                position_secs: current.position_secs,
                duration_secs: current.duration_secs,
                completed: false,
            });
            return Ok(Json(NextEpisodeResponse {
                episode: Some(item),
                reason: "resume".to_string(),
            }));
        }
    }

    // Find the highest-index completed episode
    let max_completed_index = series
        .episodes
        .iter()
        .filter(|ep| all_progress.iter().any(|p| p.episode_id == ep.id && p.completed))
        .map(|ep| ep.index)
        .max();

    if let Some(idx) = max_completed_index {
        let next_idx = idx + 1;
        if let Some(next_ep) = series.episodes.iter().find(|e| e.index == next_idx) {
            return Ok(Json(NextEpisodeResponse {
                episode: Some(build_episode_item(next_ep, &series.id, &state)),
                reason: "next".to_string(),
            }));
        } else {
            return Ok(Json(NextEpisodeResponse {
                episode: None,
                reason: "all_watched".to_string(),
            }));
        }
    }

    // Nothing watched — start from the beginning
    let first = &series.episodes[0];
    Ok(Json(NextEpisodeResponse {
        episode: Some(build_episode_item(first, &series.id, &state)),
        reason: "first".to_string(),
    }))
}

async fn continue_watching(State(state): State<Arc<AppState>>) -> ApiResult<Json<Vec<ContinueWatchingItem>>> {
    let lib = state.library.read().await;
    // 3 queries total for the entire endpoint
    let all_meta = state.db.get_all_series_metadata();
    let all_progress = state.db.get_all_progress_map();
    let all_ep_meta = state.db.get_all_episode_metadata();

    let mut items: Vec<(String, ContinueWatchingItem)> = Vec::new();

    for series in lib.series.values() {
        if series.episodes.is_empty() {
            continue;
        }

        // Filter progress entries for this series
        let series_progress: Vec<&crate::db::WatchProgress> = series
            .episodes
            .iter()
            .filter_map(|ep| all_progress.get(&ep.id))
            .collect();

        if series_progress.is_empty() {
            continue;
        }

        let meta = all_meta.get(&series.id);
        let series_title = meta
            .and_then(|m| m.title.clone())
            .unwrap_or_else(|| series.title.clone());

        // Check for in-progress episode
        let in_progress = series_progress.iter().find(|p| !p.completed && p.position_secs > 0.0);

        if let Some(current) = in_progress {
            if let Some(ep) = series.episodes.iter().find(|e| e.id == current.episode_id) {
                let mut item =
                    build_episode_item_cached(ep, &series.id, &state.media_path, &all_progress, &all_ep_meta);
                item.progress = Some(EpisodeProgress {
                    position_secs: current.position_secs,
                    duration_secs: current.duration_secs,
                    completed: false,
                });
                items.push((
                    current.updated_at.clone(),
                    ContinueWatchingItem {
                        series_id: series.id.clone(),
                        series_title,
                        has_art: series.art.is_some(),
                        has_backdrop: series.backdrop.is_some(),
                        next_episode: item,
                        reason: "resume".to_string(),
                    },
                ));
                continue;
            }
        }

        // Find the highest-index completed episode
        let max_completed_index = series
            .episodes
            .iter()
            .filter(|ep| all_progress.get(&ep.id).map(|p| p.completed).unwrap_or(false))
            .map(|ep| ep.index)
            .max();

        if let Some(idx) = max_completed_index {
            let next_idx = idx + 1;
            if let Some(next_ep) = series.episodes.iter().find(|e| e.index == next_idx) {
                let most_recent = series_progress
                    .iter()
                    .map(|p| p.updated_at.as_str())
                    .max()
                    .unwrap_or("")
                    .to_string();
                items.push((
                    most_recent,
                    ContinueWatchingItem {
                        series_id: series.id.clone(),
                        series_title,
                        has_art: series.art.is_some(),
                        has_backdrop: series.backdrop.is_some(),
                        next_episode: build_episode_item_cached(
                            next_ep,
                            &series.id,
                            &state.media_path,
                            &all_progress,
                            &all_ep_meta,
                        ),
                        reason: "next".to_string(),
                    },
                ));
            }
        }
    }

    items.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(Json(items.into_iter().map(|(_, item)| item).collect()))
}

async fn get_series_art(State(state): State<Arc<AppState>>, Path(series_id): Path<String>) -> ApiResult<Response> {
    // Prefer artwork stored in the DB; fall back to legacy filesystem files.
    if let Some((ct, bytes)) = state.db.get_artwork(&series_id, "art") {
        return Ok(([(header::CONTENT_TYPE, ct)], bytes).into_response());
    }
    let lib = state.library.read().await;
    let series = lib
        .find_series(&series_id)
        .ok_or_else(|| ApiError::not_found("Series not found"))?;
    let art_rel = series
        .art
        .as_ref()
        .ok_or_else(|| ApiError::not_found("No artwork available for this series"))?;
    let art_path = safe_media_path(&state.media_path, art_rel)?;
    let content_type = mime_guess::from_path(&art_path).first_or_octet_stream().to_string();
    let data = tokio::fs::read(&art_path)
        .await
        .map_err(|_| ApiError::internal("Failed to read file"))?;
    Ok(([(header::CONTENT_TYPE, content_type)], data).into_response())
}

/// Check if a file needs remuxing (MKV → MP4) for Apple device compatibility
pub fn needs_remux(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_lowercase().as_str(), "mkv" | "avi" | "webm" | "flv"))
        .unwrap_or(false)
}

/// One probe result: encoder name, its ffmpeg args, and whether it's usable on this machine.
pub type EncoderProbe = (&'static str, &'static str, Result<(), String>);

/// Probe results cached once per process — the hardware doesn't change mid-run, but the
/// *selection* (which probed encoder we actually use) can differ per `start_server` call
/// depending on the ServerConfig override.
static PROBED_ENCODERS: std::sync::LazyLock<Vec<EncoderProbe>> = std::sync::LazyLock::new(|| {
    // `-pix_fmt yuv420p` is critical: most files that trigger transcoding are 10-bit
    // HEVC / VP9 / AV1 sources, and h264_nvenc / qsv / amf are 8-bit-only. Without
    // this flag ffmpeg tries to encode 10-bit H.264 and the HW encoder aborts with
    // "10 bit encode not supported". yuv420p is also Apple TV's preferred playback
    // format. h264_qsv wants nv12 specifically.
    const CANDIDATES: &[(&str, &str)] = &[
        ("h264_nvenc", "-pix_fmt yuv420p -preset p4 -rc vbr -cq 23 -b:v 0"),
        (
            "h264_qsv",
            "-pix_fmt nv12 -preset medium -global_quality 23 -look_ahead 1",
        ),
        (
            "h264_amf",
            "-pix_fmt yuv420p -quality balanced -rc cqp -qp_i 22 -qp_p 22",
        ),
        ("h264_videotoolbox", "-pix_fmt yuv420p -q:v 55"),
    ];
    CANDIDATES
        .iter()
        .map(|(name, args)| (*name, *args, probe_encoder(name)))
        .collect()
});

/// Run the probe against every candidate, returning all results so the caller can
/// surface them to the user log. Cached via PROBED_ENCODERS so re-invocations are free.
pub fn probe_all_encoders() -> &'static [EncoderProbe] {
    &PROBED_ENCODERS
}

/// Resolve an encoder override string (from config/env) into a concrete encoder tuple.
/// Accepts: auto | nvenc | qsv | amf | videotoolbox | software | libx264, plus aliases.
/// Returns `(encoder, args, selection_log_message)` — the log message is meant for the
/// UI so users can see why a particular encoder was chosen or rejected.
pub fn resolve_encoder(override_value: Option<&str>) -> ((&'static str, &'static str), String) {
    let probes = probe_all_encoders();
    let choice = override_value.map(|s| s.trim().to_lowercase());
    match choice.as_deref() {
        Some("auto") | Some("") | None => {
            let enc = pick_first_ok(probes);
            (enc, format!("Transcoding encoder: {} (auto)", label_for(enc.0)))
        }
        Some("software") | Some("libx264") => (
            ("libx264", "-pix_fmt yuv420p -crf 18 -preset fast"),
            "Transcoding encoder: software (libx264) — explicitly requested".to_string(),
        ),
        Some(other) => {
            let full_name = match other {
                "nvenc" | "h264_nvenc" => "h264_nvenc",
                "qsv" | "h264_qsv" => "h264_qsv",
                "amf" | "h264_amf" => "h264_amf",
                "videotoolbox" | "vt" | "h264_videotoolbox" => "h264_videotoolbox",
                _ => {
                    let enc = pick_first_ok(probes);
                    return (
                        enc,
                        format!("Unknown encoder '{other}', falling back to {}", label_for(enc.0)),
                    );
                }
            };
            if let Some(found) = probes.iter().find(|(n, _, r)| *n == full_name && r.is_ok()) {
                (
                    (found.0, found.1),
                    format!("Transcoding encoder: {} (requested)", label_for(full_name)),
                )
            } else {
                let enc = pick_first_ok(probes);
                (
                    enc,
                    format!(
                        "Requested encoder {full_name} isn't available on this machine; falling back to {}",
                        label_for(enc.0)
                    ),
                )
            }
        }
    }
}

fn pick_first_ok(probes: &[EncoderProbe]) -> (&'static str, &'static str) {
    for (name, args, result) in probes {
        if result.is_ok() {
            return (*name, *args);
        }
    }
    ("libx264", "-pix_fmt yuv420p -crf 18 -preset fast")
}

/// Probe whether an encoder is usable by doing a tiny test encode. `-encoders` only
/// tells us the encoder is compiled in, not that the driver/device is actually present.
fn probe_encoder(name: &str) -> Result<(), String> {
    // 1 second at 25 fps → 25 frames at 256x256. Larger than NVENC's 48x48 min and
    // produces enough frames that any encoder that's going to work will accept it.
    let result = crate::media::ffmpeg_command()
        .args(["-hide_banner", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "testsrc=duration=1:size=256x256:rate=25"])
        .args(["-c:v", name])
        .args(["-f", "null", "-"])
        .output();
    let out = result.map_err(|e| format!("spawn failed: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let reason = stderr
        .lines()
        .find(|l| !l.trim().is_empty() && !l.contains("Conversion failed") && !l.contains("Error opening output"))
        .unwrap_or("unknown error")
        .trim()
        .to_string();
    Err(reason)
}

/// Human-readable name of an encoder id.
pub fn label_for(encoder: &str) -> String {
    match encoder {
        "h264_nvenc" => "NVIDIA NVENC (h264_nvenc)".to_string(),
        "h264_qsv" => "Intel QuickSync (h264_qsv)".to_string(),
        "h264_amf" => "AMD AMF (h264_amf)".to_string(),
        "h264_videotoolbox" => "Apple VideoToolbox (h264_videotoolbox)".to_string(),
        other => format!("software ({other})"),
    }
}

/// Check if the video stream needs transcoding (HEVC 10-bit, VP9, etc.)
/// Returns ("copy", ...) for compatible codecs, or the server's configured transcode
/// encoder for codecs that need to be re-encoded.
pub fn detect_video_codec(
    path: &std::path::Path,
    transcode_encoder: (&'static str, &'static str),
) -> (&'static str, &'static str) {
    let output = crate::media::ffprobe_command()
        .arg("-v")
        .arg("quiet")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=codec_name,pix_fmt")
        .arg("-of")
        .arg("csv=p=0")
        .arg(path)
        .output();

    if let Ok(output) = output {
        let info = String::from_utf8_lossy(&output.stdout);
        let info = info.trim();
        let is_hevc = info.starts_with("hevc") || info.starts_with("h265");
        let is_10bit = info.contains("10le") || info.contains("10be");
        let is_vp9 = info.starts_with("vp9");
        let is_av1 = info.starts_with("av1");

        if is_vp9 || is_av1 || (is_hevc && is_10bit) {
            tracing::info!("Video needs transcoding: {info}");
            return transcode_encoder;
        }
        if is_hevc {
            tracing::info!("Video is HEVC 8-bit, using copy: {info}");
            return ("copy", "");
        }
    }
    ("copy", "")
}

/// Serve a file with byte-range support
async fn serve_file(
    path: std::path::PathBuf,
    headers: &HeaderMap,
    file_size: u64,
    content_type: &str,
) -> ApiResult<Response> {
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| parse_range(s, file_size));

    match range {
        Some((start, end)) => {
            let length = end - start + 1;
            let mut file = tokio::fs::File::open(&path)
                .await
                .map_err(|_| ApiError::internal("Failed to read file"))?;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|_| ApiError::internal("Failed to seek"))?;
            let stream = ReaderStream::new(file.take(length));
            let body = axum::body::Body::from_stream(stream);
            Ok(Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CONTENT_LENGTH, length.to_string())
                .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{file_size}"))
                .header(header::ACCEPT_RANGES, "bytes")
                .body(body)
                .unwrap())
        }
        None => {
            let file = tokio::fs::File::open(&path)
                .await
                .map_err(|_| ApiError::internal("Failed to read file"))?;
            let stream = ReaderStream::new(file);
            let body = axum::body::Body::from_stream(stream);
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CONTENT_LENGTH, file_size.to_string())
                .header(header::ACCEPT_RANGES, "bytes")
                .body(body)
                .unwrap())
        }
    }
}

/// Stream a video file with byte-range support for seeking.
/// MKV files are remuxed on-the-fly to fragmented MP4 via ffmpeg.
async fn stream_episode(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let lib = state.library.read().await;
    let (_series, episode) = lib
        .find_episode(&episode_id)
        .ok_or_else(|| ApiError::not_found("Episode not found"))?;
    let file_path = safe_media_path(&state.media_path, &episode.path)?;

    // Check if a pre-remuxed MP4 sibling exists (from background remux)
    if needs_remux(&file_path) {
        let stem = file_path.file_stem().unwrap_or_default().to_string_lossy();
        let parent = file_path.parent().unwrap();
        let mp4_sibling = parent.join(format!("{stem}.mp4"));
        let legacy_mp4 = parent.join(".remux").join(format!("{stem}.mp4"));
        let cached = if mp4_sibling.exists() {
            Some(mp4_sibling)
        } else if legacy_mp4.exists() {
            Some(legacy_mp4)
        } else {
            None
        };
        if let Some(cached_path) = cached {
            let file_size = cached_path
                .metadata()
                .map(|m| m.len())
                .map_err(|_| ApiError::not_found("Video file not found"))?;
            return serve_file(cached_path, &headers, file_size, "video/mp4").await;
        }
    }

    let file_size = file_path
        .metadata()
        .map(|m| m.len())
        .map_err(|_| ApiError::not_found("Video file not found"))?;

    // MKV/AVI/WebM: remux on-the-fly via ffmpeg
    if needs_remux(&file_path) {
        // Register as active stream (prevents background task from deleting the MKV)
        let stem = file_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        if let Ok(mut streams) = state.active_streams.lock() {
            streams.insert(stem.clone());
        }
        // Note: we should unregister when streaming ends, but since the stream is consumed
        // by the client, the active_streams entry is cleaned up by the cleanup task.
        return stream_remuxed(file_path, headers, file_size, state.clone()).await;
    }

    let content_type = mime_guess::from_path(&file_path).first_or_octet_stream().to_string();
    serve_file(file_path, &headers, file_size, &content_type).await
}

/// Remux a non-MP4 file to MP4 via ffmpeg.
/// - If a sibling .mp4 or legacy .remux/*.mp4 exists, serve it with byte-range support.
/// - Otherwise, stream a fragmented MP4 from ffmpeg stdout while caching alongside the original.
async fn stream_remuxed(
    file_path: std::path::PathBuf,
    headers: HeaderMap,
    _file_size: u64,
    state: Arc<AppState>,
) -> ApiResult<Response> {
    let parent = file_path.parent().unwrap();
    let stem = file_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let sibling_mp4 = parent.join(format!("{stem}.mp4"));
    let legacy_mp4 = parent.join(".remux").join(format!("{stem}.mp4"));

    // Check for existing cached MP4 (sibling or legacy .remux/)
    let cached = if sibling_mp4.exists() {
        Some(sibling_mp4.clone())
    } else if legacy_mp4.exists() {
        Some(legacy_mp4)
    } else {
        None
    };

    if let Some(cached_path) = cached {
        let file_size = cached_path
            .metadata()
            .map(|m| m.len())
            .map_err(|_| ApiError::internal("Cached file not found"))?;
        return serve_file(cached_path, &headers, file_size, "video/mp4").await;
    }

    // Mark as remuxing so background/on-demand paths don't start a duplicate
    if let Ok(mut set) = state.remuxing.lock() {
        if set.contains(&stem) {
            // Another path is already remuxing — wait for the .tmp file to appear,
            // then let the client retry when it's ready
            return Err(ApiError::internal("File is already being remuxed, try again shortly"));
        }
        set.insert(stem.clone());
    }

    // Not cached — stream directly from ffmpeg as fragmented MP4 (instant start)
    // and tee the output to a cache file for future plays
    let (video_codec, video_extra) = detect_video_codec(&file_path, state.transcode_encoder);
    state.log(&format!(
        "Streaming+caching: {} (video: {video_codec})",
        file_path.file_name().unwrap().to_string_lossy()
    ));

    let mut cmd = crate::media::ffmpeg_command();
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("warning")
        .arg("-i")
        .arg(&file_path)
        .arg("-c:v")
        .arg(video_codec);

    // Add quality/speed settings for transcoding
    if video_codec != "copy" {
        for part in video_extra.split_whitespace() {
            cmd.arg(part);
        }
    }

    let mut child = cmd
        .arg("-c:a")
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
        .arg("frag_keyframe+empty_moov+default_base_moof")
        .arg("-f")
        .arg("mp4")
        .arg("pipe:1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            state.log(&format!("Failed to spawn ffmpeg: {e}"));
            ApiError::internal("ffmpeg not available — install ffmpeg to play MKV files")
        })?;

    // Log stderr in background — forward to the UI log so the user can see ffmpeg warnings
    if let Some(stderr) = child.stderr.take() {
        let err_state = state.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = String::new();
            std::io::BufReader::new(stderr).read_to_string(&mut buf).ok();
            if !buf.is_empty() {
                for line in buf.lines().filter(|l| !l.trim().is_empty()).take(20) {
                    err_state.log(&format!("ffmpeg: {line}"));
                }
            }
        });
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ApiError::internal("Failed to capture ffmpeg output"))?;

    // Tee: read from ffmpeg, send to HTTP response AND write to sibling .mp4
    let tmp_path = parent.join(format!("{stem}.mp4.tmp"));
    let final_path = sibling_mp4;
    let cache_file = std::fs::File::create(&tmp_path).ok();

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(32);

    let remuxing_cleanup = state.remuxing.clone();
    let active_streams_cleanup = state.active_streams.clone();
    let log_state = state.clone();
    let stem_cleanup = stem;
    std::thread::spawn(move || {
        // Drop guard ensures `remuxing` and `active_streams` sets are cleaned up even on panic
        struct Cleanup {
            remuxing: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
            active_streams: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
            stem: String,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(mut set) = self.remuxing.lock() {
                    set.remove(&self.stem);
                }
                if let Ok(mut set) = self.active_streams.lock() {
                    set.remove(&self.stem);
                }
            }
        }
        let _cleanup = Cleanup {
            remuxing: remuxing_cleanup,
            active_streams: active_streams_cleanup,
            stem: stem_cleanup,
        };

        use std::io::{Read, Write};
        let mut stdout = stdout;
        let mut cache = cache_file;
        let mut buf = vec![0u8; 256 * 1024];
        let mut client_disconnected = false;
        loop {
            match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = buf[..n].to_vec();
                    if let Some(ref mut f) = cache {
                        let _ = f.write_all(&chunk);
                    }
                    if tx.blocking_send(Ok(chunk)).is_err() {
                        // Client disconnected — stop pumping but let ffmpeg finish so cache completes
                        client_disconnected = true;
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                    break;
                }
            }
        }
        // Even if the client left, drain stdout into the cache so the cached file is complete.
        if client_disconnected {
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Some(ref mut f) = cache {
                            let _ = f.write_all(&buf[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
        }

        let ffmpeg_ok = child.wait().map(|s| s.success()).unwrap_or(false);
        if let Some(ref mut f) = cache {
            let _ = f.flush();
        }
        drop(cache);

        if !ffmpeg_ok {
            log_state.log(&format!(
                "ffmpeg remux failed for {}; discarding cache",
                file_path.file_name().unwrap_or_default().to_string_lossy()
            ));
            let _ = std::fs::remove_file(&tmp_path);
            return;
        }

        if std::fs::rename(&tmp_path, &final_path).is_ok() {
            log_state.log(&format!(
                "Remux cached: {}",
                final_path.file_name().unwrap().to_string_lossy()
            ));
            // Delete original only after confirming the MP4 is readable and non-empty
            let mp4_ok = final_path.metadata().map(|m| m.len() > 0).unwrap_or(false);
            if mp4_ok && file_path.exists() && std::fs::remove_file(&file_path).is_ok() {
                log_state.log(&format!(
                    "Deleted original: {}",
                    file_path.file_name().unwrap().to_string_lossy()
                ));
            }
        } else {
            let _ = std::fs::remove_file(&tmp_path);
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let body = axum::body::Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/mp4")
        .header(header::TRANSFER_ENCODING, "chunked")
        .body(body)
        .unwrap())
}

fn parse_range(range_header: &str, file_size: u64) -> Option<(u64, u64)> {
    let range_str = range_header.strip_prefix("bytes=")?;
    let parts: Vec<&str> = range_str.splitn(2, '-').collect();
    if parts.len() != 2 {
        return None;
    }

    let start: u64 = if parts[0].is_empty() {
        // Suffix range: -500 means last 500 bytes
        let suffix: u64 = parts[1].parse().ok()?;
        file_size.saturating_sub(suffix)
    } else {
        parts[0].parse().ok()?
    };

    let end: u64 = if parts[1].is_empty() {
        file_size - 1
    } else {
        parts[1].parse().ok()?
    };

    if start <= end && start < file_size {
        Some((start, end.min(file_size - 1)))
    } else {
        None
    }
}

async fn get_progress(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
) -> ApiResult<Json<Option<EpisodeProgress>>> {
    let progress = state.db.get_progress(&episode_id).map(|p| EpisodeProgress {
        position_secs: p.position_secs,
        duration_secs: p.duration_secs,
        completed: p.completed,
    });
    Ok(Json(progress))
}

async fn update_progress(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
    Json(body): Json<ProgressUpdate>,
) -> ApiResult<StatusCode> {
    state
        .db
        .update_progress(&episode_id, body.position_secs, body.duration_secs)
        .map_err(|_| ApiError::internal("Failed to update progress"))?;
    Ok(StatusCode::OK)
}

async fn get_all_progress(State(state): State<Arc<AppState>>) -> Json<Vec<crate::db::WatchProgress>> {
    Json(state.db.get_all_progress())
}

async fn delete_progress(State(state): State<Arc<AppState>>, Path(episode_id): Path<String>) -> ApiResult<StatusCode> {
    state
        .db
        .delete_progress(&episode_id)
        .map_err(|_| ApiError::internal("Failed to delete progress"))?;
    Ok(StatusCode::OK)
}

async fn delete_series_progress(
    State(state): State<Arc<AppState>>,
    Path(series_id): Path<String>,
) -> ApiResult<StatusCode> {
    let lib = state.library.read().await;
    let series = lib
        .find_series(&series_id)
        .ok_or_else(|| ApiError::not_found("Series not found"))?;

    let episode_ids: Vec<String> = series.episodes.iter().map(|e| e.id.clone()).collect();
    state
        .db
        .delete_series_progress(&episode_ids)
        .map_err(|_| ApiError::internal("Failed to delete series progress"))?;
    Ok(StatusCode::OK)
}

async fn get_series_backdrop(State(state): State<Arc<AppState>>, Path(series_id): Path<String>) -> ApiResult<Response> {
    if let Some((ct, bytes)) = state.db.get_artwork(&series_id, "backdrop") {
        return Ok(([(header::CONTENT_TYPE, ct)], bytes).into_response());
    }
    let lib = state.library.read().await;
    let series = lib
        .find_series(&series_id)
        .ok_or_else(|| ApiError::not_found("Series not found"))?;
    let backdrop_rel = series
        .backdrop
        .as_ref()
        .ok_or_else(|| ApiError::not_found("No backdrop available for this series"))?;
    let backdrop_path = safe_media_path(&state.media_path, backdrop_rel)?;
    let content_type = mime_guess::from_path(&backdrop_path)
        .first_or_octet_stream()
        .to_string();
    let data = tokio::fs::read(&backdrop_path)
        .await
        .map_err(|_| ApiError::internal("Failed to read file"))?;
    Ok(([(header::CONTENT_TYPE, content_type)], data).into_response())
}

/// Serve a thumbnail image for an episode (generated via ffmpeg).
/// - If the file already exists, serve it immediately.
/// - If another request is already generating this thumbnail, wait for it instead
///   of spawning a duplicate ffmpeg (TV app loads a whole season of thumbs at once).
/// - Caps concurrent thumbnail ffmpegs via `thumb_semaphore` so we don't spawn one
///   ffmpeg per episode when a series page opens.
/// - Memoizes generation failures for the lifetime of the process, so a file that
///   ffmpeg can't turn into a thumbnail doesn't cost a fresh ffmpeg run on every
///   page load.
async fn get_episode_thumbnail(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
) -> ApiResult<Response> {
    let thumb_dir = state.media_path.join(".thumbnails");
    let thumb_path = thumb_dir.join(format!("{episode_id}.jpg"));

    if !thumb_path.exists() {
        if let Some(reason) = previous_thumb_failure(&state, &episode_id)? {
            return Err(thumb_failure_response(&reason));
        }

        generate_episode_thumbnail(&state, &episode_id, &thumb_dir, &thumb_path).await?;
    }

    if !thumb_path.exists() {
        return Err(thumb_failure_response("thumbnail missing after generation"));
    }

    let data = tokio::fs::read(&thumb_path)
        .await
        .map_err(|_| ApiError::internal("Failed to read file"))?;

    Ok(([(header::CONTENT_TYPE, "image/jpeg".to_string())], data).into_response())
}

/// Return `Some(reason)` if a previous ffmpeg attempt for this id failed in this
/// process lifetime. Lets the handler short-circuit without re-running ffmpeg.
fn previous_thumb_failure(state: &Arc<AppState>, id: &str) -> ApiResult<Option<String>> {
    let map = state
        .thumb_failures
        .lock()
        .map_err(|_| ApiError::internal("lock poisoned"))?;
    Ok(map.get(id).cloned())
}

fn record_thumb_failure(state: &Arc<AppState>, id: &str, reason: &str) {
    if let Ok(mut map) = state.thumb_failures.lock() {
        map.insert(id.to_string(), reason.to_string());
    }
}

fn clear_thumb_failure(state: &Arc<AppState>, id: &str) {
    if let Ok(mut map) = state.thumb_failures.lock() {
        map.remove(id);
    }
}

/// Standardized response when thumbnail generation is known to have failed or
/// times out. Uses 502 because the upstream (ffmpeg) failed to produce media —
/// not a 404 (the episode exists) and not a plain 500 (it's deterministic).
fn thumb_failure_response(reason: &str) -> (StatusCode, Json<ApiError>) {
    tracing::debug!("Thumbnail unavailable: {reason}");
    (
        StatusCode::BAD_GATEWAY,
        Json(ApiError {
            error: "Thumbnail unavailable".to_string(),
            code: 502,
            detail: Some(reason.to_string()),
        }),
    )
}

async fn generate_episode_thumbnail(
    state: &Arc<AppState>,
    episode_id: &str,
    thumb_dir: &std::path::Path,
    thumb_path: &std::path::Path,
) -> ApiResult<()> {
    // Claim the in-flight slot for this id. If someone else owns it, wait for the
    // file to appear rather than spawning a duplicate ffmpeg.
    let claimed = {
        let mut set = state
            .generating_thumbs
            .lock()
            .map_err(|_| ApiError::internal("lock poisoned"))?;
        if set.contains(episode_id) {
            false
        } else {
            set.insert(episode_id.to_string());
            true
        }
    };

    if !claimed {
        for _ in 0..120 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if thumb_path.exists() {
                return Ok(());
            }
            if let Some(reason) = previous_thumb_failure(state, episode_id)? {
                return Err(thumb_failure_response(&reason));
            }
        }
        return Err(thumb_failure_response("generation timed out waiting for peer"));
    }

    // We own the slot — ensure we release it even on error.
    struct Guard<'a> {
        set: &'a Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
        id: &'a str,
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            if let Ok(mut s) = self.set.lock() {
                s.remove(self.id);
            }
        }
    }
    let _guard = Guard {
        set: &state.generating_thumbs,
        id: episode_id,
    };

    let video_path = {
        let lib = state.library.read().await;
        let (_series, episode) = lib
            .find_episode(episode_id)
            .ok_or_else(|| ApiError::not_found("Episode not found"))?;
        safe_media_path(&state.media_path, &episode.path)?
    };

    tokio::fs::create_dir_all(thumb_dir)
        .await
        .map_err(|_| ApiError::internal("Failed to create thumbnail directory"))?;

    let _permit = state
        .thumb_semaphore
        .acquire()
        .await
        .map_err(|_| ApiError::internal("Thumbnail semaphore closed"))?;

    let duration = crate::media::probe_duration(&video_path).unwrap_or(300.0);
    let timestamp = (duration * 0.1).min(30.0);

    let vp = video_path;
    let tp = thumb_path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || crate::media::generate_thumbnail(&vp, &tp, timestamp))
        .await
        .map_err(|_| ApiError::internal("thumbnail task panicked"))?;

    match result {
        Ok(()) => {
            clear_thumb_failure(state, episode_id);
            Ok(())
        }
        Err(e) => {
            let reason = e.to_string();
            state.log(&format!("Thumbnail generation failed for {episode_id}: {reason}"));
            record_thumb_failure(state, episode_id, &reason);
            Err(thumb_failure_response(&reason))
        }
    }
}

#[derive(Serialize)]
struct SubtitleInfo {
    language: String,
    label: String,
}

fn language_label(code: &str) -> String {
    match code {
        "en" => "English".to_string(),
        "sv" => "Swedish".to_string(),
        "de" => "German".to_string(),
        "fr" => "French".to_string(),
        "es" => "Spanish".to_string(),
        "no" => "Norwegian".to_string(),
        "da" => "Danish".to_string(),
        "fi" => "Finnish".to_string(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                None => other.to_string(),
            }
        }
    }
}

async fn list_subtitles(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
) -> ApiResult<Json<Vec<SubtitleInfo>>> {
    let lib = state.library.read().await;
    let (_series, episode) = lib
        .find_episode(&episode_id)
        .ok_or_else(|| ApiError::not_found("Episode not found"))?;

    let subs: Vec<SubtitleInfo> = episode
        .subtitles
        .iter()
        .map(|s| SubtitleInfo {
            label: language_label(&s.language),
            language: s.language.clone(),
        })
        .collect();

    Ok(Json(subs))
}

async fn get_subtitle(
    State(state): State<Arc<AppState>>,
    Path((episode_id, language)): Path<(String, String)>,
) -> ApiResult<Response> {
    let lib = state.library.read().await;
    let (_series, episode) = lib
        .find_episode(&episode_id)
        .ok_or_else(|| ApiError::not_found("Episode not found"))?;

    let sub = episode
        .subtitles
        .iter()
        .find(|s| s.language == language)
        .ok_or_else(|| ApiError::not_found("Subtitle language not found"))?;

    let sub_path = safe_media_path(&state.media_path, &sub.path)?;
    let srt_content = tokio::fs::read_to_string(&sub_path)
        .await
        .map_err(|_| ApiError::internal("Failed to read subtitle file"))?;

    let vtt = crate::subtitle::srt_to_webvtt(&srt_content);

    Ok(([(header::CONTENT_TYPE, "text/vtt".to_string())], vtt).into_response())
}

#[derive(Serialize)]
struct FetchMetadataResponse {
    downloaded: usize,
    message: String,
}

/// Trigger TMDB metadata/art fetch for all series
async fn fetch_metadata(State(state): State<Arc<AppState>>) -> ApiResult<Json<FetchMetadataResponse>> {
    let client = state.tmdb.as_ref().ok_or_else(|| {
        state.log("TMDB fetch requested but no API key configured");
        ApiError::unavailable("TMDB API key not configured")
    })?;

    let series_info: Vec<(String, String, bool, bool, Option<u64>)> = {
        let lib = state.library.read().await;
        lib.series
            .values()
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
    };

    let total = series_info.len();
    let log_state = state.clone();
    let debug_state = state.clone();
    let downloaded = crate::tmdb::fetch_all_metadata(
        client,
        &state.db,
        &state.media_path,
        series_info,
        move |msg| log_state.log(msg),
        move |msg| debug_state.debug(msg),
    )
    .await;

    // Rescan library to pick up new art files
    match crate::library::Library::scan(&state.media_path) {
        Ok(lib) => *state.library.write().await = lib,
        Err(e) => state.log(&format!("Rescan after metadata fetch failed: {e}")),
    }

    Ok(Json(FetchMetadataResponse {
        downloaded,
        message: format!("Processed {total} series, downloaded art for {downloaded}"),
    }))
}

/// Get cast/guest stars for an episode (cached in DB, fetched from TMDB on first request)
async fn get_episode_credits(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
) -> ApiResult<Json<crate::tmdb::EpisodeCredits>> {
    let tmdb_client = state
        .tmdb
        .as_ref()
        .ok_or_else(|| ApiError::unavailable("TMDB API key not configured"))?;

    // Find the episode and its series
    let (series_id, season, episode) = {
        let lib = state.library.read().await;
        let (series, ep) = lib
            .find_episode(&episode_id)
            .ok_or_else(|| ApiError::not_found("Episode not found"))?;

        let season = ep
            .season_number
            .ok_or_else(|| ApiError::not_found("Episode has no season/episode number — cannot look up credits"))?;
        let episode = ep
            .episode_number
            .ok_or_else(|| ApiError::not_found("Episode has no episode number — cannot look up credits"))?;
        (series.id.clone(), season, episode)
    };

    // Check cache first
    if let Some(cached_json) = state.db.get_episode_credits(&series_id, season, episode) {
        let credits: crate::tmdb::EpisodeCredits =
            serde_json::from_str(&cached_json).map_err(|_| ApiError::internal("Corrupt credits cache"))?;
        return Ok(Json(credits));
    }

    // Need TMDB ID for the series
    let tmdb_id = state
        .db
        .get_series_metadata(&series_id)
        .and_then(|m| m.tmdb_id)
        .ok_or_else(|| ApiError::not_found("No TMDB metadata for this series"))?;

    // Fetch from TMDB
    let credits = tmdb_client
        .get_episode_credits(tmdb_id, season, episode)
        .await
        .map_err(|e| ApiError::internal(&format!("TMDB request failed: {e}")))?;

    // Cache in DB
    if let Ok(json) = serde_json::to_string(&credits) {
        let _ = state.db.save_episode_credits(&series_id, season, episode, &json);
    }

    Ok(Json(credits))
}

/// Prepare an episode for playback — triggers remux if needed, returns status
#[derive(Serialize)]
struct PrepareResponse {
    ready: bool,
    needs_remux: bool,
    remuxing: bool,
    progress_percent: Option<u32>,
}

async fn prepare_episode(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
) -> ApiResult<Json<PrepareResponse>> {
    let lib = state.library.read().await;
    let (_series, episode) = lib
        .find_episode(&episode_id)
        .ok_or_else(|| ApiError::not_found("Episode not found"))?;
    let file_path = safe_media_path(&state.media_path, &episode.path)?;

    // If it's already MP4, it's ready
    if !needs_remux(&file_path) {
        return Ok(Json(PrepareResponse {
            ready: true,
            needs_remux: false,
            remuxing: false,
            progress_percent: None,
        }));
    }

    let stem = file_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let parent = file_path.parent().unwrap();
    let mp4_path = parent.join(format!("{stem}.mp4"));
    let tmp_path = parent.join(format!("{stem}.mp4.tmp"));
    let legacy_mp4 = parent.join(".remux").join(format!("{stem}.mp4"));

    // Already remuxed?
    if mp4_path.exists() || legacy_mp4.exists() {
        return Ok(Json(PrepareResponse {
            ready: true,
            needs_remux: true,
            remuxing: false,
            progress_percent: Some(100),
        }));
    }

    // Currently remuxing? Report progress based on file size ratio.
    // A .tmp file only counts as "in progress" if something on this server is actually
    // working on it (state.remuxing contains the stem). Otherwise it's an orphan from a
    // prior crashed run — clean it up and fall through to kick off a fresh remux.
    let is_remuxing = state.remuxing.lock().map(|s| s.contains(&stem)).unwrap_or(false);
    if !is_remuxing && tmp_path.exists() {
        state.log(&format!(
            "Found orphaned {} with no active remux — cleaning up",
            tmp_path.file_name().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_file(&tmp_path);
    }

    if is_remuxing {
        let progress = if let (Ok(tmp_meta), Ok(src_meta)) = (tmp_path.metadata(), file_path.metadata()) {
            let src_size = src_meta.len();
            let tmp_size = tmp_meta.len();
            // Rough estimate: remuxed MP4 is ~similar size to source
            if src_size > 0 {
                Some((tmp_size as f64 / src_size as f64 * 100.0).min(99.0) as u32)
            } else {
                Some(0)
            }
        } else {
            Some(0)
        };
        return Ok(Json(PrepareResponse {
            ready: false,
            needs_remux: true,
            remuxing: true,
            progress_percent: progress,
        }));
    }

    // Not remuxing yet — kick it off now
    let file_path_clone = file_path.to_path_buf();
    let tmp_clone = tmp_path.clone();
    let mp4_clone = mp4_path.clone();
    let stem_clone = stem.clone();

    // Mark as remuxing
    if let Ok(mut set) = state.remuxing.lock() {
        set.insert(stem.clone());
    }

    drop(lib);

    let remuxing_ref = state.remuxing.clone();
    let log_state = state.clone();
    tokio::task::spawn_blocking(move || {
        let (video_codec, video_extra) = detect_video_codec(&file_path_clone, log_state.transcode_encoder);
        log_state.log(&format!(
            "On-demand remux: {} (video: {video_codec})",
            file_path_clone.file_name().unwrap().to_string_lossy()
        ));

        let mut cmd = crate::media::ffmpeg_command();
        cmd.arg("-hide_banner")
            .arg("-loglevel")
            .arg("warning")
            .arg("-i")
            .arg(&file_path_clone)
            .arg("-c:v")
            .arg(video_codec);
        if video_codec != "copy" {
            for part in video_extra.split_whitespace() {
                cmd.arg(part);
            }
        }
        let output = cmd
            .arg("-c:a")
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
            .output();

        match output {
            Ok(result) if result.status.success() => {
                if std::fs::rename(&tmp_clone, &mp4_clone).is_ok() {
                    log_state.log(&format!(
                        "On-demand remux complete: {}",
                        mp4_clone.file_name().unwrap().to_string_lossy()
                    ));
                    // Delete original
                    let _ = std::fs::remove_file(&file_path_clone);
                }
            }
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                log_state.log(&format!(
                    "On-demand remux failed: {}",
                    stderr.lines().next().unwrap_or("unknown")
                ));
                let _ = std::fs::remove_file(&tmp_clone);
            }
            Err(e) => {
                log_state.log(&format!("ffmpeg not available: {e}"));
            }
        }

        // Unmark
        if let Ok(mut set) = remuxing_ref.lock() {
            set.remove(&stem_clone);
        }
    });

    Ok(Json(PrepareResponse {
        ready: false,
        needs_remux: true,
        remuxing: true,
        progress_percent: Some(0),
    }))
}

/// Trigger remux for all MKV episodes in a series.
/// Episodes are enqueued and processed one at a time so we don't spawn N ffmpeg
/// processes in parallel — that would peg the CPU and disk.
async fn remux_series(
    State(state): State<Arc<AppState>>,
    Path(series_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let lib = state.library.read().await;
    let series = lib
        .find_series(&series_id)
        .ok_or_else(|| ApiError::not_found("Series not found"))?;

    state.log(&format!(
        "Manual remux requested for series '{}' ({} episodes)",
        series.title,
        series.episodes.len()
    ));

    // Manual trigger is also a "retry" signal — clear any abandoned-failure flags on
    // this series so previously-given-up episodes get re-queued.
    state.db.retry_remux_failures_for_series(&series_id);

    // Collect jobs synchronously, then kick off a single worker that processes them
    // sequentially. Reserving each stem in `state.remuxing` up front prevents the
    // background task, on-demand prepare and streaming paths from duplicating work.
    let mut jobs: Vec<(std::path::PathBuf, std::path::PathBuf, String)> = Vec::new();
    let mut already_mp4 = 0;
    let mut already_done = 0;
    let mut already_running = 0;
    for ep in &series.episodes {
        let ep_path = state.media_path.join(&ep.path);
        if !needs_remux(&ep_path) {
            already_mp4 += 1;
            continue;
        }
        let stem = ep_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let parent = match ep_path.parent() {
            Some(p) => p,
            None => continue,
        };
        let mp4_path = parent.join(format!("{stem}.mp4"));
        if mp4_path.exists() {
            already_done += 1;
            continue;
        }
        let tmp_path = parent.join(format!("{stem}.mp4.tmp"));
        // .tmp only counts as "running" if this server process has the stem reserved.
        // Otherwise it's an orphan — a manual Remux All is the user's "please retry", so
        // sweep it aside and queue fresh.
        if tmp_path.exists() {
            let active = state.remuxing.lock().map(|s| s.contains(&stem)).unwrap_or(false);
            if active {
                already_running += 1;
                continue;
            }
            if std::fs::remove_file(&tmp_path).is_err() {
                already_running += 1;
                continue;
            }
            state.log(&format!(
                "Cleaned up orphaned {}",
                tmp_path.file_name().unwrap().to_string_lossy()
            ));
        }
        let reserved = {
            let mut set = match state.remuxing.lock() {
                Ok(s) => s,
                Err(_) => continue,
            };
            if set.contains(&stem) {
                false
            } else {
                set.insert(stem.clone());
                true
            }
        };
        if !reserved {
            already_running += 1;
            continue;
        }
        jobs.push((ep_path, mp4_path, stem));
    }

    let triggered = jobs.len();
    state.log(&format!(
        "Manual remux queued {triggered} episode(s) (already MP4: {already_mp4}, already remuxed: {already_done}, already running: {already_running}) — processing one at a time"
    ));
    drop(lib);

    if !jobs.is_empty() {
        let worker_state = state.clone();
        tokio::spawn(async move {
            let total = jobs.len();
            for (index, (ep_path, mp4_path, stem)) in jobs.into_iter().enumerate() {
                let tmp_path = mp4_path.with_extension("mp4.tmp");
                let log_state = worker_state.clone();
                let remuxing_ref = worker_state.remuxing.clone();
                let ep_path_clone = ep_path.clone();
                let tmp_clone = tmp_path.clone();
                let mp4_clone = mp4_path.clone();
                let stem_clone = stem.clone();

                let result = tokio::task::spawn_blocking(move || {
                    let (video_codec, video_extra) = detect_video_codec(&ep_path_clone, log_state.transcode_encoder);
                    log_state.log(&format!(
                        "Batch remux [{}/{total}]: {} (video: {video_codec})",
                        index + 1,
                        ep_path_clone.file_name().unwrap().to_string_lossy()
                    ));
                    let mut cmd = crate::media::ffmpeg_command();
                    cmd.arg("-hide_banner")
                        .arg("-loglevel")
                        .arg("warning")
                        .arg("-i")
                        .arg(&ep_path_clone)
                        .arg("-c:v")
                        .arg(video_codec);
                    if video_codec != "copy" {
                        for part in video_extra.split_whitespace() {
                            cmd.arg(part);
                        }
                    }
                    let output = cmd
                        .arg("-c:a")
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
                        .output();
                    match output {
                        Ok(result) if result.status.success() => {
                            if std::fs::rename(&tmp_clone, &mp4_clone).is_ok() {
                                log_state.log(&format!(
                                    "Batch remux complete [{}/{total}]: {}",
                                    index + 1,
                                    mp4_clone.file_name().unwrap().to_string_lossy()
                                ));
                                let _ = std::fs::remove_file(&ep_path_clone);
                            }
                        }
                        Ok(result) => {
                            let stderr = String::from_utf8_lossy(&result.stderr);
                            log_state.log(&format!(
                                "Batch remux failed [{}/{total}]: {}",
                                index + 1,
                                stderr.lines().next().unwrap_or("unknown error")
                            ));
                            let _ = std::fs::remove_file(&tmp_clone);
                        }
                        Err(e) => {
                            log_state.log(&format!("ffmpeg not available: {e}"));
                            let _ = std::fs::remove_file(&tmp_clone);
                        }
                    }
                    if let Ok(mut set) = remuxing_ref.lock() {
                        set.remove(&stem_clone);
                    }
                })
                .await;

                if let Err(e) = result {
                    worker_state.log(&format!("Batch remux worker panicked on '{stem}': {e}"));
                    if let Ok(mut set) = worker_state.remuxing.lock() {
                        set.remove(&stem);
                    }
                }
            }
            worker_state.log(&format!("Batch remux queue drained ({total} episode(s))"));
        });
    }

    Ok(Json(serde_json::json!({ "triggered": triggered })))
}

/// Get all watched episodes across all series
#[derive(Serialize)]
struct WatchedEpisode {
    episode_id: String,
    series_id: String,
    series_title: String,
    folder_name: String,
    episode_label: String,
    title: String,
    filename: String,
    size_bytes: u64,
    percent_watched: u32,
    format: String,
}

async fn get_watched_episodes(State(state): State<Arc<AppState>>) -> Json<Vec<WatchedEpisode>> {
    let lib = state.library.read().await;
    let all_progress = state.db.get_all_progress_map();
    let all_ep_meta = state.db.get_all_episode_metadata();

    let mut watched = Vec::new();
    let all_series_meta = state.db.get_all_series_metadata();

    for series in lib.series.values() {
        let display_title = all_series_meta
            .get(&series.id)
            .and_then(|m| m.title.clone())
            .unwrap_or_else(|| series.title.clone());

        for ep in &series.episodes {
            if let Some(progress) = all_progress.get(&ep.id) {
                if progress.completed {
                    let tmdb_meta = ep
                        .season_number
                        .zip(ep.episode_number)
                        .and_then(|(s, e)| all_ep_meta.get(&(series.id.clone(), s, e)));

                    let title = tmdb_meta
                        .and_then(|m| m.title.clone())
                        .unwrap_or_else(|| ep.title.clone());

                    let label = if let (Some(s), Some(e)) = (ep.season_number, ep.episode_number) {
                        format!("S{s} E{e}")
                    } else {
                        format!("Episode {}", ep.index + 1)
                    };

                    let percent = if progress.duration_secs > 0.0 {
                        ((progress.position_secs / progress.duration_secs) * 100.0).min(100.0) as u32
                    } else {
                        100
                    };

                    let format = std::path::Path::new(&ep.path)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("unknown")
                        .to_lowercase();

                    let filename = std::path::Path::new(&ep.path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();

                    watched.push(WatchedEpisode {
                        episode_id: ep.id.clone(),
                        series_id: series.id.clone(),
                        series_title: display_title.clone(),
                        folder_name: series.title.clone(),
                        episode_label: label,
                        title,
                        filename,
                        size_bytes: ep.size_bytes,
                        percent_watched: percent,
                        format,
                    });
                }
            }
        }
    }

    Json(watched)
}

/// Trigger an immediate library rescan + TMDB metadata fetch
async fn rescan_library(State(state): State<Arc<AppState>>) -> ApiResult<Json<serde_json::Value>> {
    state.log("Manual rescan triggered");

    match crate::library::Library::scan(&state.media_path) {
        Ok(lib) => {
            let series_count = lib.series.len();
            let episode_count: usize = lib.series.values().map(|s| s.episodes.len()).sum();

            // Fetch TMDB metadata for series missing it
            if let Some(ref client) = state.tmdb {
                let needs_meta: Vec<_> = lib
                    .series
                    .values()
                    .filter(|s| {
                        s.art.is_none() || s.backdrop.is_none() || state.db.get_series_metadata(&s.id).is_none()
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

                if !needs_meta.is_empty() {
                    let count = needs_meta.len();
                    state.debug(&format!("Fetching TMDB metadata for {count} series..."));
                    let log_state = state.clone();
                    let debug_state = state.clone();
                    let downloaded = crate::tmdb::fetch_all_metadata(
                        client,
                        &state.db,
                        &state.media_path,
                        needs_meta,
                        move |msg| log_state.log(msg),
                        move |msg| debug_state.debug(msg),
                    )
                    .await;
                    if downloaded > 0 {
                        state.log(&format!("Downloaded artwork for {downloaded} series"));
                    }
                    // Rescan again to pick up new art
                    if let Ok(updated) = crate::library::Library::scan(&state.media_path) {
                        *state.library.write().await = updated;
                        state.log(&format!(
                            "Rescan complete: {series_count} series, {episode_count} episodes"
                        ));
                        return Ok(Json(
                            serde_json::json!({ "series": series_count, "episodes": episode_count }),
                        ));
                    }
                }
            }

            *state.library.write().await = lib;
            state.log(&format!(
                "Rescan complete: {series_count} series, {episode_count} episodes"
            ));
            Ok(Json(
                serde_json::json!({ "series": series_count, "episodes": episode_count }),
            ))
        }
        Err(e) => {
            state.log(&format!("Rescan failed: {e}"));
            Err(ApiError::internal(&format!("Rescan failed: {e}")))
        }
    }
}

/// Delete a series and all its files
async fn delete_series(
    State(state): State<Arc<AppState>>,
    Path(series_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let series_title = {
        let lib = state.library.read().await;
        let series = lib
            .find_series(&series_id)
            .ok_or_else(|| ApiError::not_found("Series not found"))?;
        series.title.clone()
    };

    // Delete the series folder
    let series_dir = state.media_path.join(&series_title);
    if series_dir.exists() {
        std::fs::remove_dir_all(&series_dir).map_err(|e| ApiError::internal(&format!("Failed to delete: {e}")))?;
    }

    // Clean up DB metadata + artwork + TMDB attempt rows
    state.db.delete_series_metadata(&series_id);
    state.db.delete_artwork(&series_id);
    state.db.clear_tmdb_failure(&series_id);

    // Rescan
    if let Ok(lib) = crate::library::Library::scan(&state.media_path) {
        *state.library.write().await = lib;
    }

    state.log(&format!("Deleted series: {series_title}"));
    Ok(Json(serde_json::json!({ "deleted": series_title })))
}

/// Delete a single episode file
async fn delete_episode(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let file_path = {
        let lib = state.library.read().await;
        let (_series, episode) = lib
            .find_episode(&episode_id)
            .ok_or_else(|| ApiError::not_found("Episode not found"))?;
        state.media_path.join(&episode.path)
    };

    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Delete the video file and all related files (srt, mp4 sibling, tmp)
    if let (Some(parent), Some(stem)) = (file_path.parent(), file_path.file_stem()) {
        let stem_str = stem.to_string_lossy();
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if let Some(entry_stem) = entry_path.file_stem() {
                    let entry_stem_str = entry_stem.to_string_lossy();
                    // Delete files that share the same stem or start with "stem."
                    // This catches: video.mkv, video.mp4, video.srt, video.en.srt, video.mp4.tmp
                    if entry_stem_str == stem_str
                        || entry_stem_str.starts_with(&format!("{stem_str}."))
                        || entry_path
                            .to_string_lossy()
                            .starts_with(&format!("{}/{stem_str}.", parent.display()))
                    {
                        let _ = std::fs::remove_file(&entry_path);
                    }
                }
            }
        }
    }

    // Clean up progress and any remux-failure record for this episode
    let _ = state.db.delete_progress(&episode_id);
    state.db.retry_remux_failures(Some(&episode_id));

    // Rescan
    if let Ok(lib) = crate::library::Library::scan(&state.media_path) {
        *state.library.write().await = lib;
    }

    state.log(&format!("Deleted episode: {filename}"));
    Ok(Json(serde_json::json!({ "deleted": filename })))
}

// ============================================================
// Movie endpoints
// ============================================================

#[derive(Serialize)]
struct MovieListItem {
    id: String,
    title: String,
    year: Option<String>,
    path: String,
    size_bytes: u64,
    has_art: bool,
    has_backdrop: bool,
    has_metadata: bool,
    overview: Option<String>,
    genres: Option<String>,
    rating: Option<f64>,
    runtime_minutes: Option<u32>,
    tagline: Option<String>,
    progress: Option<EpisodeProgress>,
}

async fn list_movies(State(state): State<Arc<AppState>>) -> Json<Vec<MovieListItem>> {
    let lib = state.library.read().await;
    let all_progress = state.db.get_all_progress_map();
    let mut items: Vec<MovieListItem> = lib
        .movies
        .values()
        .map(|m| {
            let meta = state.db.get_movie_metadata(&m.id);
            let prog = all_progress.get(&m.id).map(|p| EpisodeProgress {
                position_secs: p.position_secs,
                duration_secs: p.duration_secs,
                completed: p.completed,
            });
            MovieListItem {
                id: m.id.clone(),
                title: meta
                    .as_ref()
                    .and_then(|x| x.title.clone())
                    .unwrap_or_else(|| m.title.clone()),
                year: meta
                    .as_ref()
                    .and_then(|x| x.release_date.as_ref())
                    .and_then(|d| d.get(..4).map(|s| s.to_string()))
                    .or_else(|| m.year.clone()),
                path: m.path.clone(),
                size_bytes: m.size_bytes,
                has_art: m.art.is_some() || state.db.has_artwork(&m.id, "art"),
                has_backdrop: m.backdrop.is_some() || state.db.has_artwork(&m.id, "backdrop"),
                has_metadata: meta.is_some(),
                overview: meta.as_ref().and_then(|x| x.overview.clone()),
                genres: meta.as_ref().and_then(|x| x.genres.clone()),
                rating: meta.as_ref().and_then(|x| x.rating),
                runtime_minutes: meta.as_ref().and_then(|x| x.runtime_minutes),
                tagline: meta.as_ref().and_then(|x| x.tagline.clone()),
                progress: prog,
            }
        })
        .collect();
    items.sort_by_key(|m| m.title.to_lowercase());
    Json(items)
}

#[derive(Serialize)]
struct MovieDetail {
    id: String,
    title: String,
    year: Option<String>,
    path: String,
    size_bytes: u64,
    has_art: bool,
    has_backdrop: bool,
    has_metadata: bool,
    overview: Option<String>,
    genres: Option<String>,
    rating: Option<f64>,
    runtime_minutes: Option<u32>,
    tagline: Option<String>,
    progress: Option<EpisodeProgress>,
    has_external_subtitles: bool,
}

async fn get_movie(State(state): State<Arc<AppState>>, Path(movie_id): Path<String>) -> ApiResult<Json<MovieDetail>> {
    let lib = state.library.read().await;
    let m = lib
        .find_movie(&movie_id)
        .ok_or_else(|| ApiError::not_found("Movie not found"))?;
    let meta = state.db.get_movie_metadata(&m.id);
    let prog = state.db.get_progress(&m.id).map(|p| EpisodeProgress {
        position_secs: p.position_secs,
        duration_secs: p.duration_secs,
        completed: p.completed,
    });
    Ok(Json(MovieDetail {
        id: m.id.clone(),
        title: meta
            .as_ref()
            .and_then(|x| x.title.clone())
            .unwrap_or_else(|| m.title.clone()),
        year: meta
            .as_ref()
            .and_then(|x| x.release_date.as_ref())
            .and_then(|d| d.get(..4).map(|s| s.to_string()))
            .or_else(|| m.year.clone()),
        path: m.path.clone(),
        size_bytes: m.size_bytes,
        has_art: m.art.is_some() || state.db.has_artwork(&m.id, "art"),
        has_backdrop: m.backdrop.is_some() || state.db.has_artwork(&m.id, "backdrop"),
        has_metadata: meta.is_some(),
        overview: meta.as_ref().and_then(|x| x.overview.clone()),
        genres: meta.as_ref().and_then(|x| x.genres.clone()),
        rating: meta.as_ref().and_then(|x| x.rating),
        runtime_minutes: meta.as_ref().and_then(|x| x.runtime_minutes),
        tagline: meta.as_ref().and_then(|x| x.tagline.clone()),
        progress: prog,
        has_external_subtitles: !m.subtitles.is_empty(),
    }))
}

async fn stream_movie(
    State(state): State<Arc<AppState>>,
    Path(movie_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let file_path = {
        let lib = state.library.read().await;
        let m = lib
            .find_movie(&movie_id)
            .ok_or_else(|| ApiError::not_found("Movie not found"))?;
        safe_media_path(&state.media_path, &m.path)?
    };

    // If a pre-remuxed sibling .mp4 exists for a non-MP4 source, prefer it
    if needs_remux(&file_path) {
        let stem = file_path.file_stem().unwrap_or_default().to_string_lossy();
        let parent = file_path.parent().unwrap();
        let mp4_sibling = parent.join(format!("{stem}.mp4"));
        if mp4_sibling.exists() {
            let file_size = mp4_sibling
                .metadata()
                .map(|m| m.len())
                .map_err(|_| ApiError::not_found("Video file not found"))?;
            return serve_file(mp4_sibling, &headers, file_size, "video/mp4").await;
        }
    }

    let file_size = file_path
        .metadata()
        .map(|m| m.len())
        .map_err(|_| ApiError::not_found("Video file not found"))?;

    if needs_remux(&file_path) {
        let stem = file_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        if let Ok(mut streams) = state.active_streams.lock() {
            streams.insert(stem.clone());
        }
        return stream_remuxed(file_path, headers, file_size, state.clone()).await;
    }

    let content_type = mime_guess::from_path(&file_path).first_or_octet_stream().to_string();
    serve_file(file_path, &headers, file_size, &content_type).await
}

async fn get_movie_thumbnail(State(state): State<Arc<AppState>>, Path(movie_id): Path<String>) -> ApiResult<Response> {
    // If the movie has art (poster), serve that instead of generating a frame thumb.
    let art_rel = {
        let lib = state.library.read().await;
        let m = lib
            .find_movie(&movie_id)
            .ok_or_else(|| ApiError::not_found("Movie not found"))?;
        m.art.clone()
    };

    if let Some(rel) = art_rel {
        let path = safe_media_path(&state.media_path, &rel)?;
        let ct = mime_guess::from_path(&path).first_or_octet_stream().to_string();
        let data = tokio::fs::read(&path)
            .await
            .map_err(|_| ApiError::internal("Failed to read poster"))?;
        return Ok(([(header::CONTENT_TYPE, ct)], data).into_response());
    }

    // Fall back to an ffmpeg-generated thumb of the movie at ~10% / 30s.
    let thumb_dir = state.media_path.join(".thumbnails");
    let thumb_path = thumb_dir.join(format!("{movie_id}.jpg"));
    if !thumb_path.exists() {
        if let Some(reason) = previous_thumb_failure(&state, &movie_id)? {
            return Err(thumb_failure_response(&reason));
        }

        let video_path = {
            let lib = state.library.read().await;
            let m = lib
                .find_movie(&movie_id)
                .ok_or_else(|| ApiError::not_found("Movie not found"))?;
            safe_media_path(&state.media_path, &m.path)?
        };
        tokio::fs::create_dir_all(&thumb_dir)
            .await
            .map_err(|_| ApiError::internal("Failed to create thumbnail directory"))?;
        let _permit = state
            .thumb_semaphore
            .acquire()
            .await
            .map_err(|_| ApiError::internal("Thumbnail semaphore closed"))?;
        let duration = crate::media::probe_duration(&video_path).unwrap_or(300.0);
        let timestamp = (duration * 0.1).min(30.0);
        let vp = video_path.clone();
        let tp = thumb_path.clone();
        let result = tokio::task::spawn_blocking(move || crate::media::generate_thumbnail(&vp, &tp, timestamp))
            .await
            .map_err(|_| ApiError::internal("thumbnail task panicked"))?;
        match result {
            Ok(()) => clear_thumb_failure(&state, &movie_id),
            Err(e) => {
                let reason = e.to_string();
                state.log(&format!("Thumbnail generation failed for movie {movie_id}: {reason}"));
                record_thumb_failure(&state, &movie_id, &reason);
                return Err(thumb_failure_response(&reason));
            }
        }
    }
    if !thumb_path.exists() {
        return Err(thumb_failure_response("thumbnail missing after generation"));
    }
    let data = tokio::fs::read(&thumb_path)
        .await
        .map_err(|_| ApiError::internal("Failed to read file"))?;
    Ok(([(header::CONTENT_TYPE, "image/jpeg".to_string())], data).into_response())
}

async fn get_movie_art(State(state): State<Arc<AppState>>, Path(movie_id): Path<String>) -> ApiResult<Response> {
    if let Some((ct, bytes)) = state.db.get_artwork(&movie_id, "art") {
        return Ok(([(header::CONTENT_TYPE, ct)], bytes).into_response());
    }
    let lib = state.library.read().await;
    let m = lib
        .find_movie(&movie_id)
        .ok_or_else(|| ApiError::not_found("Movie not found"))?;
    let rel = m
        .art
        .as_ref()
        .ok_or_else(|| ApiError::not_found("No poster for this movie"))?;
    let path = safe_media_path(&state.media_path, rel)?;
    let ct = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::internal("Failed to read poster"))?;
    Ok(([(header::CONTENT_TYPE, ct)], data).into_response())
}

async fn get_movie_backdrop(State(state): State<Arc<AppState>>, Path(movie_id): Path<String>) -> ApiResult<Response> {
    if let Some((ct, bytes)) = state.db.get_artwork(&movie_id, "backdrop") {
        return Ok(([(header::CONTENT_TYPE, ct)], bytes).into_response());
    }
    let lib = state.library.read().await;
    let m = lib
        .find_movie(&movie_id)
        .ok_or_else(|| ApiError::not_found("Movie not found"))?;
    let rel = m
        .backdrop
        .as_ref()
        .ok_or_else(|| ApiError::not_found("No backdrop for this movie"))?;
    let path = safe_media_path(&state.media_path, rel)?;
    let ct = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::internal("Failed to read backdrop"))?;
    Ok(([(header::CONTENT_TYPE, ct)], data).into_response())
}

async fn get_movie_progress(
    State(state): State<Arc<AppState>>,
    Path(movie_id): Path<String>,
) -> ApiResult<Json<Option<EpisodeProgress>>> {
    // Confirm the movie exists
    {
        let lib = state.library.read().await;
        lib.find_movie(&movie_id)
            .ok_or_else(|| ApiError::not_found("Movie not found"))?;
    }
    let progress = state.db.get_progress(&movie_id).map(|p| EpisodeProgress {
        position_secs: p.position_secs,
        duration_secs: p.duration_secs,
        completed: p.completed,
    });
    Ok(Json(progress))
}

async fn update_movie_progress(
    State(state): State<Arc<AppState>>,
    Path(movie_id): Path<String>,
    Json(body): Json<ProgressUpdate>,
) -> ApiResult<StatusCode> {
    state
        .db
        .update_progress(&movie_id, body.position_secs, body.duration_secs)
        .map_err(|_| ApiError::internal("Failed to update progress"))?;
    Ok(StatusCode::OK)
}

async fn delete_movie_progress(
    State(state): State<Arc<AppState>>,
    Path(movie_id): Path<String>,
) -> ApiResult<StatusCode> {
    state
        .db
        .delete_progress(&movie_id)
        .map_err(|_| ApiError::internal("Failed to clear progress"))?;
    Ok(StatusCode::OK)
}

async fn prepare_movie(
    State(state): State<Arc<AppState>>,
    Path(movie_id): Path<String>,
) -> ApiResult<Json<PrepareResponse>> {
    let lib = state.library.read().await;
    let m = lib
        .find_movie(&movie_id)
        .ok_or_else(|| ApiError::not_found("Movie not found"))?;
    let file_path = safe_media_path(&state.media_path, &m.path)?;

    if !needs_remux(&file_path) {
        return Ok(Json(PrepareResponse {
            ready: true,
            needs_remux: false,
            remuxing: false,
            progress_percent: None,
        }));
    }

    let stem = file_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let parent = file_path.parent().unwrap();
    let mp4_path = parent.join(format!("{stem}.mp4"));
    let tmp_path = parent.join(format!("{stem}.mp4.tmp"));

    if mp4_path.exists() {
        return Ok(Json(PrepareResponse {
            ready: true,
            needs_remux: true,
            remuxing: false,
            progress_percent: Some(100),
        }));
    }

    let is_remuxing = state.remuxing.lock().map(|s| s.contains(&stem)).unwrap_or(false);
    if !is_remuxing && tmp_path.exists() {
        state.log(&format!(
            "Found orphaned {} with no active remux — cleaning up",
            tmp_path.file_name().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_file(&tmp_path);
    }

    if is_remuxing {
        let progress = if let (Ok(tmp_meta), Ok(src_meta)) = (tmp_path.metadata(), file_path.metadata()) {
            let src_size = src_meta.len();
            let tmp_size = tmp_meta.len();
            if src_size > 0 {
                Some((tmp_size as f64 / src_size as f64 * 100.0).min(99.0) as u32)
            } else {
                Some(0)
            }
        } else {
            Some(0)
        };
        return Ok(Json(PrepareResponse {
            ready: false,
            needs_remux: true,
            remuxing: true,
            progress_percent: progress,
        }));
    }

    // Not remuxing yet — kick it off now
    let file_path_clone = file_path.to_path_buf();
    let tmp_clone = tmp_path.clone();
    let mp4_clone = mp4_path.clone();
    let stem_clone = stem.clone();

    if let Ok(mut set) = state.remuxing.lock() {
        set.insert(stem.clone());
    }
    drop(lib);

    let remuxing_ref = state.remuxing.clone();
    let log_state = state.clone();
    tokio::task::spawn_blocking(move || {
        let (video_codec, video_extra) = detect_video_codec(&file_path_clone, log_state.transcode_encoder);
        log_state.log(&format!(
            "On-demand movie remux: {} (video: {video_codec})",
            file_path_clone.file_name().unwrap().to_string_lossy()
        ));
        let mut cmd = crate::media::ffmpeg_command();
        cmd.arg("-hide_banner")
            .arg("-loglevel")
            .arg("warning")
            .arg("-i")
            .arg(&file_path_clone)
            .arg("-c:v")
            .arg(video_codec);
        if video_codec != "copy" {
            for part in video_extra.split_whitespace() {
                cmd.arg(part);
            }
        }
        let output = cmd
            .arg("-c:a")
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
            .output();
        match output {
            Ok(result) if result.status.success() => {
                if std::fs::rename(&tmp_clone, &mp4_clone).is_ok() {
                    log_state.log(&format!(
                        "On-demand movie remux complete: {}",
                        mp4_clone.file_name().unwrap().to_string_lossy()
                    ));
                    let _ = std::fs::remove_file(&file_path_clone);
                }
            }
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                log_state.log(&format!(
                    "On-demand movie remux failed: {}",
                    stderr.lines().next().unwrap_or("unknown")
                ));
                let _ = std::fs::remove_file(&tmp_clone);
            }
            Err(e) => {
                log_state.log(&format!("ffmpeg not available: {e}"));
            }
        }
        if let Ok(mut set) = remuxing_ref.lock() {
            set.remove(&stem_clone);
        }
    });

    Ok(Json(PrepareResponse {
        ready: false,
        needs_remux: true,
        remuxing: true,
        progress_percent: Some(0),
    }))
}

async fn list_movie_subtitles(
    State(state): State<Arc<AppState>>,
    Path(movie_id): Path<String>,
) -> ApiResult<Json<Vec<SubtitleInfo>>> {
    let lib = state.library.read().await;
    let m = lib
        .find_movie(&movie_id)
        .ok_or_else(|| ApiError::not_found("Movie not found"))?;
    let subs: Vec<SubtitleInfo> = m
        .subtitles
        .iter()
        .map(|s| SubtitleInfo {
            language: s.language.clone(),
            label: language_label(&s.language),
        })
        .collect();
    Ok(Json(subs))
}

async fn get_movie_subtitle(
    State(state): State<Arc<AppState>>,
    Path((movie_id, language)): Path<(String, String)>,
) -> ApiResult<Response> {
    let sub_rel = {
        let lib = state.library.read().await;
        let m = lib
            .find_movie(&movie_id)
            .ok_or_else(|| ApiError::not_found("Movie not found"))?;
        m.subtitles
            .iter()
            .find(|s| s.language == language)
            .map(|s| s.path.clone())
            .ok_or_else(|| ApiError::not_found("Subtitle not found"))?
    };
    let path = safe_media_path(&state.media_path, &sub_rel)?;
    let srt = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| ApiError::internal("Failed to read subtitle file"))?;
    let vtt = crate::subtitle::srt_to_webvtt(&srt);
    Ok(([(header::CONTENT_TYPE, "text/vtt".to_string())], vtt).into_response())
}

async fn delete_movie(
    State(state): State<Arc<AppState>>,
    Path(movie_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let file_path = {
        let lib = state.library.read().await;
        let m = lib
            .find_movie(&movie_id)
            .ok_or_else(|| ApiError::not_found("Movie not found"))?;
        state.media_path.join(&m.path)
    };
    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Delete the video + any related files sharing the same stem
    if let (Some(parent), Some(stem)) = (file_path.parent(), file_path.file_stem()) {
        let stem_str = stem.to_string_lossy();
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if let Some(entry_stem) = entry_path.file_stem() {
                    let entry_stem_str = entry_stem.to_string_lossy();
                    if entry_stem_str == stem_str || entry_stem_str.starts_with(&format!("{stem_str}.")) {
                        let _ = std::fs::remove_file(&entry_path);
                    }
                }
            }
        }
    }

    let _ = state.db.delete_progress(&movie_id);
    state.db.delete_movie_metadata(&movie_id);
    state.db.delete_artwork(&movie_id);
    state.db.clear_tmdb_failure(&movie_id);
    state.db.retry_remux_failures(Some(&movie_id));

    if let Ok(lib) = crate::library::Library::scan(&state.media_path) {
        *state.library.write().await = lib;
    }

    state.log(&format!("Deleted movie: {filename}"));
    Ok(Json(serde_json::json!({ "deleted": filename })))
}

/// Get person detail with biography and filmography from TMDB
async fn get_person(
    State(state): State<Arc<AppState>>,
    Path(person_id): Path<u64>,
) -> ApiResult<Json<crate::tmdb::PersonDetail>> {
    let tmdb_client = state
        .tmdb
        .as_ref()
        .ok_or_else(|| ApiError::unavailable("TMDB API key not configured"))?;

    let person = tmdb_client
        .get_person_detail(person_id)
        .await
        .map_err(|e| ApiError::internal(&format!("TMDB request failed: {e}")))?
        .ok_or_else(|| ApiError::not_found("Person not found"))?;

    Ok(Json(person))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::fs;
    use tempfile::TempDir;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    /// Create a test AppState with a temp directory containing one series with two episodes.
    /// Returns (TempDir, Arc<AppState>, series_id, episode_ids).
    fn setup_test_state() -> (TempDir, Arc<AppState>, String, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        let series_dir = dir.path().join("TestShow");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("S01E01.mp4"), b"fake-video-data-one").unwrap();
        fs::write(series_dir.join("S01E02.mp4"), b"fake-video-data-two").unwrap();
        fs::write(series_dir.join("poster.jpg"), b"fake-image").unwrap();

        let db = crate::db::Database::new(dir.path()).unwrap();
        let lib = crate::library::Library::scan(dir.path()).unwrap();

        let series = lib.series.values().next().unwrap();
        let series_id = series.id.clone();
        let episode_ids: Vec<String> = series.episodes.iter().map(|e| e.id.clone()).collect();

        let state = Arc::new(AppState {
            library: RwLock::new(lib),
            db,
            media_path: dir.path().to_path_buf(),
            tmdb: None,
            active_streams: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            remuxing: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            generating_thumbs: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            thumb_failures: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            thumb_semaphore: Arc::new(tokio::sync::Semaphore::new(2)),
            transcode_encoder: ("libx264", "-pix_fmt yuv420p -crf 18 -preset fast"),
            encoder_label: "software (libx264)".to_string(),
            debug_logging: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            log: None,
        });

        (dir, state, series_id, episode_ids)
    }

    fn app(state: Arc<AppState>) -> Router {
        create_router(state)
    }

    async fn body_to_bytes(body: Body) -> Vec<u8> {
        body.collect().await.unwrap().to_bytes().to_vec()
    }

    async fn body_to_json(body: Body) -> serde_json::Value {
        let bytes = body_to_bytes(body).await;
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn list_series_returns_list() {
        let (_dir, state, _series_id, _ep_ids) = setup_test_state();
        let response = app(state)
            .oneshot(Request::builder().uri("/api/series").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["title"], "TestShow");
        assert_eq!(arr[0]["episode_count"], 2);
        assert!(arr[0]["has_art"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn get_series_returns_detail_with_episodes() {
        let (_dir, state, series_id, _ep_ids) = setup_test_state();
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/series/{series_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["title"], "TestShow");
        let episodes = json["episodes"].as_array().unwrap();
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0]["title"], "Episode 1");
        assert_eq!(episodes[1]["title"], "Episode 2");
    }

    #[tokio::test]
    async fn get_series_returns_404_for_unknown() {
        let (_dir, state, _series_id, _ep_ids) = setup_test_state();
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/api/series/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn next_episode_returns_first_when_nothing_watched() {
        let (_dir, state, series_id, _ep_ids) = setup_test_state();
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/series/{series_id}/next"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["reason"], "first");
        assert_eq!(json["episode"]["index"], 0);
    }

    #[tokio::test]
    async fn next_episode_returns_next_after_completed() {
        let (_dir, state, series_id, ep_ids) = setup_test_state();
        // Mark first episode as completed (>=90%)
        state.db.update_progress(&ep_ids[0], 950.0, 1000.0).unwrap();

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/series/{series_id}/next"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["reason"], "next");
        assert_eq!(json["episode"]["index"], 1);
    }

    #[tokio::test]
    async fn next_episode_resume_in_progress() {
        let (_dir, state, series_id, ep_ids) = setup_test_state();
        // Set progress on first ep without completing
        state.db.update_progress(&ep_ids[0], 300.0, 1000.0).unwrap();

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/series/{series_id}/next"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        assert_eq!(json["reason"], "resume");
        assert_eq!(json["episode"]["index"], 0);
    }

    #[tokio::test]
    async fn post_and_get_progress_roundtrip() {
        let (_dir, state, _series_id, ep_ids) = setup_test_state();
        let ep_id = &ep_ids[0];

        // POST progress
        let response = app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/episodes/{ep_id}/progress"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"position_secs": 120.0, "duration_secs": 3600.0}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // GET progress
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/episodes/{ep_id}/progress"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        assert!((json["position_secs"].as_f64().unwrap() - 120.0).abs() < f64::EPSILON);
        assert!((json["duration_secs"].as_f64().unwrap() - 3600.0).abs() < f64::EPSILON);
        assert!(!json["completed"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn stream_episode_without_range() {
        let (_dir, state, _series_id, ep_ids) = setup_test_state();
        let ep_id = &ep_ids[0];

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/episodes/{ep_id}/stream"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .get("accept-ranges")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("bytes"));
        let bytes = body_to_bytes(response.into_body()).await;
        assert_eq!(bytes, b"fake-video-data-one");
    }

    #[tokio::test]
    async fn stream_episode_with_range_header() {
        let (_dir, state, _series_id, ep_ids) = setup_test_state();
        let ep_id = &ep_ids[0];

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/episodes/{ep_id}/stream"))
                    .header("range", "bytes=0-3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert!(response.headers().get("content-range").is_some());
        let bytes = body_to_bytes(response.into_body()).await;
        assert_eq!(bytes, b"fake");
    }

    #[tokio::test]
    async fn stream_episode_404_for_unknown() {
        let (_dir, state, _series_id, _ep_ids) = setup_test_state();

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/api/episodes/nonexistent/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn art_endpoint_returns_image() {
        let (_dir, state, series_id, _ep_ids) = setup_test_state();

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/series/{series_id}/art"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let ct = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(ct.contains("jpeg") || ct.contains("jpg"));
        let bytes = body_to_bytes(response.into_body()).await;
        assert_eq!(bytes, b"fake-image");
    }

    #[tokio::test]
    async fn art_endpoint_404_for_unknown_series() {
        let (_dir, state, _series_id, _ep_ids) = setup_test_state();

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/api/series/nonexistent/art")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn art_endpoint_404_when_no_art() {
        let dir = tempfile::tempdir().unwrap();
        let series_dir = dir.path().join("NoArtShow");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("ep.mp4"), b"video").unwrap();

        let db = crate::db::Database::new(dir.path()).unwrap();
        let lib = crate::library::Library::scan(dir.path()).unwrap();
        let series_id = lib.series.values().next().unwrap().id.clone();
        let state = Arc::new(AppState {
            library: RwLock::new(lib),
            db,
            media_path: dir.path().to_path_buf(),
            tmdb: None,
            active_streams: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            remuxing: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            generating_thumbs: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            thumb_failures: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            thumb_semaphore: Arc::new(tokio::sync::Semaphore::new(2)),
            transcode_encoder: ("libx264", "-pix_fmt yuv420p -crf 18 -preset fast"),
            encoder_label: "software (libx264)".to_string(),
            debug_logging: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            log: None,
        });

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/series/{series_id}/art"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_all_progress_endpoint() {
        let (_dir, state, _series_id, ep_ids) = setup_test_state();
        state.db.update_progress(&ep_ids[0], 100.0, 1000.0).unwrap();

        let response = app(state)
            .oneshot(Request::builder().uri("/api/progress").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 1);
    }

    // --- Path traversal security tests ---

    /// Create a state with a tampered library containing paths that attempt directory traversal.
    fn setup_traversal_state() -> (TempDir, Arc<AppState>, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let series_dir = dir.path().join("EvilShow");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("ep.mp4"), b"video").unwrap();

        // Create a file outside the media dir to prove it can't be served
        fs::write(dir.path().join("secret.txt"), b"TOP SECRET").unwrap();

        let db = crate::db::Database::new(dir.path()).unwrap();
        let mut lib = crate::library::Library::scan(dir.path()).unwrap();

        let series = lib.series.values_mut().next().unwrap();
        let series_id = series.id.clone();
        let episode_id = series.episodes[0].id.clone();

        // Tamper the episode path to attempt traversal
        series.episodes[0].path = "../../../etc/passwd".to_string();
        // Tamper the art path
        series.art = Some("../secret.txt".to_string());
        // Tamper the backdrop path
        series.backdrop = Some("../secret.txt".to_string());

        let state = Arc::new(AppState {
            library: RwLock::new(lib),
            db,
            media_path: dir.path().to_path_buf(),
            tmdb: None,
            active_streams: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            remuxing: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            generating_thumbs: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            thumb_failures: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            thumb_semaphore: Arc::new(tokio::sync::Semaphore::new(2)),
            transcode_encoder: ("libx264", "-pix_fmt yuv420p -crf 18 -preset fast"),
            encoder_label: "software (libx264)".to_string(),
            debug_logging: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            log: None,
        });

        (dir, state, series_id, episode_id)
    }

    #[tokio::test]
    async fn stream_blocks_path_traversal() {
        let (_dir, state, _series_id, episode_id) = setup_traversal_state();
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/episodes/{episode_id}/stream"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should be 404 (file doesn't exist at traversal path) or 403
        assert!(
            response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::FORBIDDEN,
            "Expected 404 or 403, got {}",
            response.status()
        );
    }

    #[tokio::test]
    async fn art_blocks_path_traversal() {
        let (_dir, state, series_id, _episode_id) = setup_traversal_state();
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/series/{series_id}/art"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::FORBIDDEN,
            "Expected 404 or 403, got {}",
            response.status()
        );
        // Verify we did NOT serve the secret file
        let bytes = body_to_bytes(response.into_body()).await;
        assert_ne!(bytes, b"TOP SECRET");
    }

    #[tokio::test]
    async fn backdrop_blocks_path_traversal() {
        let (_dir, state, series_id, _episode_id) = setup_traversal_state();
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/series/{series_id}/backdrop"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::FORBIDDEN,
            "Expected 404 or 403, got {}",
            response.status()
        );
        let bytes = body_to_bytes(response.into_body()).await;
        assert_ne!(bytes, b"TOP SECRET");
    }

    #[tokio::test]
    async fn thumbnail_blocks_path_traversal() {
        let (_dir, state, _series_id, episode_id) = setup_traversal_state();
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/episodes/{episode_id}/thumbnail"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Thumbnail generation should fail because the video path is traversal
        assert!(
            response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::FORBIDDEN,
            "Expected 404 or 403, got {}",
            response.status()
        );
    }

    #[tokio::test]
    async fn stream_with_range_blocks_path_traversal() {
        let (_dir, state, _series_id, episode_id) = setup_traversal_state();
        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/episodes/{episode_id}/stream"))
                    .header("range", "bytes=0-10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::FORBIDDEN,
            "Expected 404 or 403, got {}",
            response.status()
        );
    }

    // --- Progress deletion tests ---

    #[tokio::test]
    async fn delete_single_episode_progress() {
        let (_dir, state, _series_id, ep_ids) = setup_test_state();
        let ep_id = &ep_ids[0];

        // Create progress
        state.db.update_progress(ep_id, 120.0, 3600.0).unwrap();
        assert!(state.db.get_progress(ep_id).is_some());

        // Delete via API
        let response = app(state.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/episodes/{ep_id}/progress"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Verify gone
        assert!(state.db.get_progress(ep_id).is_none());
    }

    #[tokio::test]
    async fn delete_series_progress_removes_all_episodes() {
        let (_dir, state, series_id, ep_ids) = setup_test_state();

        // Create progress for both episodes
        state.db.update_progress(&ep_ids[0], 100.0, 1000.0).unwrap();
        state.db.update_progress(&ep_ids[1], 200.0, 2000.0).unwrap();
        assert!(state.db.get_progress(&ep_ids[0]).is_some());
        assert!(state.db.get_progress(&ep_ids[1]).is_some());

        // Delete all via API
        let response = app(state.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/series/{series_id}/progress"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Verify both gone
        assert!(state.db.get_progress(&ep_ids[0]).is_none());
        assert!(state.db.get_progress(&ep_ids[1]).is_none());
    }

    #[tokio::test]
    async fn delete_series_progress_returns_404_for_unknown() {
        let (_dir, state, _series_id, _ep_ids) = setup_test_state();

        let response = app(state)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/series/nonexistent/progress")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // --- Continue watching tests ---

    #[tokio::test]
    async fn continue_watching_returns_in_progress_series() {
        // Setup: two series, mark one episode as in-progress in series A
        let dir = tempfile::tempdir().unwrap();
        let series_a_dir = dir.path().join("ShowA");
        fs::create_dir(&series_a_dir).unwrap();
        fs::write(series_a_dir.join("S01E01.mp4"), b"video-a1").unwrap();
        fs::write(series_a_dir.join("S01E02.mp4"), b"video-a2").unwrap();

        let series_b_dir = dir.path().join("ShowB");
        fs::create_dir(&series_b_dir).unwrap();
        fs::write(series_b_dir.join("S01E01.mp4"), b"video-b1").unwrap();

        let db = crate::db::Database::new(dir.path()).unwrap();
        let lib = crate::library::Library::scan(dir.path()).unwrap();

        let series_a = lib.series.values().find(|s| s.title == "ShowA").unwrap();
        let series_a_id = series_a.id.clone();
        let ep_a1_id = series_a.episodes[0].id.clone();

        let state = Arc::new(AppState {
            library: RwLock::new(lib),
            db,
            media_path: dir.path().to_path_buf(),
            tmdb: None,
            active_streams: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            remuxing: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            generating_thumbs: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            thumb_failures: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            thumb_semaphore: Arc::new(tokio::sync::Semaphore::new(2)),
            transcode_encoder: ("libx264", "-pix_fmt yuv420p -crf 18 -preset fast"),
            encoder_label: "software (libx264)".to_string(),
            debug_logging: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            log: None,
        });

        // Mark episode in series A as in-progress
        state.db.update_progress(&ep_a1_id, 300.0, 1000.0).unwrap();

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri("/api/continue-watching")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        let arr = json.as_array().unwrap();

        // Only series A should appear (series B has no progress)
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["series_id"], series_a_id);
        assert_eq!(arr[0]["series_title"], "ShowA");
        assert_eq!(arr[0]["reason"], "resume");
        assert_eq!(arr[0]["next_episode"]["id"], ep_a1_id);
    }

    #[tokio::test]
    async fn list_subtitles_returns_empty_for_no_srt_files() {
        let (_dir, state, _series_id, ep_ids) = setup_test_state();
        let ep_id = &ep_ids[0];

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/episodes/{ep_id}/subtitles"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_to_json(response.into_body()).await;
        let arr = json.as_array().unwrap();
        assert!(arr.is_empty());
    }

    #[tokio::test]
    async fn get_subtitle_returns_404_for_unknown_language() {
        let (_dir, state, _series_id, ep_ids) = setup_test_state();
        let ep_id = &ep_ids[0];

        let response = app(state)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/episodes/{ep_id}/subtitles/en"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
