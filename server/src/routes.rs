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
        .with_state(state)
}

// --- DTOs ---

#[derive(Serialize)]
struct SeriesListItem {
    id: String,
    title: String,
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
                episode_count: s.episodes.len(),
                has_art: s.art.is_some(),
                has_backdrop: s.backdrop.is_some(),
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

    result.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
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
fn needs_remux(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_lowercase().as_str(), "mkv" | "avi" | "webm" | "flv"))
        .unwrap_or(false)
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
    let file_size = file_path
        .metadata()
        .map(|m| m.len())
        .map_err(|_| ApiError::not_found("Video file not found"))?;

    // MKV/AVI/WebM: remux to fragmented MP4 via ffmpeg (no re-encoding)
    if needs_remux(&file_path) {
        return stream_remuxed(file_path, headers, file_size).await;
    }

    let content_type = mime_guess::from_path(&file_path).first_or_octet_stream().to_string();

    // Parse Range header
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| parse_range(s, file_size));

    match range {
        Some((start, end)) => {
            let length = end - start + 1;
            let mut file = tokio::fs::File::open(&file_path)
                .await
                .map_err(|_| ApiError::internal("Failed to read file"))?;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|_| ApiError::internal("Failed to read file"))?;
            let limited = file.take(length);
            let stream = ReaderStream::new(limited);
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
            let file = tokio::fs::File::open(&file_path)
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

/// Remux a non-MP4 file to fragmented MP4 via ffmpeg, streamed to the client.
/// Uses `-movflags frag_keyframe+empty_moov+faststart` so the MP4 is streamable
/// without needing the full file written first.
/// Remux a non-MP4 file to MP4 via ffmpeg, caching in .remux/ directory.
/// Serves the cached file with full byte-range support for AVPlayer seeking.
async fn stream_remuxed(
    file_path: std::path::PathBuf,
    headers: HeaderMap,
    _file_size: u64,
) -> ApiResult<Response> {
    // Cache remuxed files alongside the .thumbnails dir
    let cache_dir = file_path.parent().unwrap().join(".remux");
    let _ = std::fs::create_dir_all(&cache_dir);

    let stem = file_path.file_stem().unwrap_or_default().to_string_lossy();
    let cached_path = cache_dir.join(format!("{stem}.mp4"));

    // Remux if not already cached
    if !cached_path.exists() {
        tracing::info!("Remuxing {:?} → {:?}", file_path.file_name().unwrap(), cached_path.file_name().unwrap());

        let output = tokio::task::spawn_blocking({
            let file_path = file_path.clone();
            let cached_path = cached_path.clone();
            move || {
                std::process::Command::new("ffmpeg")
                    .arg("-hide_banner")
                    .arg("-loglevel").arg("warning")
                    .arg("-i").arg(&file_path)
                    .arg("-c:v").arg("copy")
                    .arg("-c:a").arg("aac")
                    .arg("-b:a").arg("192k")
                    .arg("-ac").arg("2")
                    .arg("-map").arg("0:v:0")
                    .arg("-map").arg("0:a:0")
                    .arg("-map").arg("0:s?")          // include all subtitle streams
                    .arg("-c:s").arg("mov_text")       // convert subs to MP4-compatible format
                    .arg("-movflags").arg("+faststart")
                    .arg("-y")
                    .arg(&cached_path)
                    .output()
            }
        })
        .await
        .map_err(|_| ApiError::internal("Remux task failed"))?
        .map_err(|e| {
            tracing::error!("Failed to run ffmpeg: {e}");
            ApiError::internal("ffmpeg not available — install ffmpeg to play MKV files")
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!("ffmpeg remux failed: {stderr}");
            // Clean up partial file
            let _ = std::fs::remove_file(&cached_path);
            return Err(ApiError::internal("Failed to remux video"));
        }

        tracing::info!("Remux complete: {:?}", cached_path.file_name().unwrap());
    }

    // Serve the cached MP4 with full byte-range support (same as regular files)
    let file_size = cached_path
        .metadata()
        .map(|m| m.len())
        .map_err(|_| ApiError::internal("Remuxed file not found"))?;

    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| parse_range(s, file_size));

    match range {
        Some((start, end)) => {
            let length = end - start + 1;
            let mut file = tokio::fs::File::open(&cached_path)
                .await
                .map_err(|_| ApiError::internal("Failed to read remuxed file"))?;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|_| ApiError::internal("Failed to seek in remuxed file"))?;
            let limited = file.take(length);
            let stream = ReaderStream::new(limited);
            let body = axum::body::Body::from_stream(stream);

            Ok(Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, "video/mp4")
                .header(header::CONTENT_LENGTH, length.to_string())
                .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{file_size}"))
                .header(header::ACCEPT_RANGES, "bytes")
                .body(body)
                .unwrap())
        }
        None => {
            let file = tokio::fs::File::open(&cached_path)
                .await
                .map_err(|_| ApiError::internal("Failed to read remuxed file"))?;
            let stream = ReaderStream::new(file);
            let body = axum::body::Body::from_stream(stream);

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "video/mp4")
                .header(header::CONTENT_LENGTH, file_size.to_string())
                .header(header::ACCEPT_RANGES, "bytes")
                .body(body)
                .unwrap())
        }
    }
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

/// Serve a thumbnail image for an episode (generated via ffmpeg)
async fn get_episode_thumbnail(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
) -> ApiResult<Response> {
    let thumb_dir = state.media_path.join(".thumbnails");
    let thumb_path = thumb_dir.join(format!("{episode_id}.jpg"));

    // Generate on-demand if missing
    if !thumb_path.exists() {
        let lib = state.library.read().await;
        let (_series, episode) = lib
            .find_episode(&episode_id)
            .ok_or_else(|| ApiError::not_found("Episode not found"))?;
        let video_path = safe_media_path(&state.media_path, &episode.path)?;

        // Create thumbnail directory
        tokio::fs::create_dir_all(&thumb_dir)
            .await
            .map_err(|_| ApiError::internal("Failed to create thumbnail directory"))?;

        // Generate thumbnail at ~10% or 30s
        let duration = crate::media::probe_duration(&video_path).unwrap_or(300.0);
        let timestamp = (duration * 0.1).min(30.0);

        let vp = video_path.clone();
        let tp = thumb_path.clone();
        tokio::task::spawn_blocking(move || crate::media::generate_thumbnail(&vp, &tp, timestamp))
            .await
            .map_err(|_| ApiError::internal("Failed to generate thumbnail"))?
            .map_err(|_| ApiError::internal("Failed to generate thumbnail"))?;
    }

    if !thumb_path.exists() {
        return Err(ApiError::not_found("Episode not found"));
    }

    let data = tokio::fs::read(&thumb_path)
        .await
        .map_err(|_| ApiError::internal("Failed to read file"))?;

    Ok(([(header::CONTENT_TYPE, "image/jpeg".to_string())], data).into_response())
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
        tracing::warn!("TMDB fetch requested but no API key configured");
        ApiError::unavailable("TMDB API key not configured")
    })?;

    let series_info: Vec<(String, String, bool, Option<u64>)> = {
        let lib = state.library.read().await;
        lib.series
            .values()
            .map(|s| (s.id.clone(), s.title.clone(), s.art.is_some(), s.tmdb_id_override))
            .collect()
    };

    let total = series_info.len();
    let downloaded = crate::tmdb::fetch_all_metadata(client, &state.db, &state.media_path, series_info).await;

    // Rescan library to pick up new art files
    match crate::library::Library::scan(&state.media_path) {
        Ok(lib) => *state.library.write().await = lib,
        Err(e) => tracing::warn!("Rescan after metadata fetch failed: {e}"),
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
    let tmdb_client = state.tmdb.as_ref().ok_or_else(|| {
        ApiError::unavailable("TMDB API key not configured")
    })?;

    // Find the episode and its series
    let (series_id, season, episode) = {
        let lib = state.library.read().await;
        let (series, ep) = lib
            .find_episode(&episode_id)
            .ok_or_else(|| ApiError::not_found("Episode not found"))?;

        let season = ep.season_number.ok_or_else(|| {
            ApiError::not_found("Episode has no season/episode number — cannot look up credits")
        })?;
        let episode = ep.episode_number.ok_or_else(|| {
            ApiError::not_found("Episode has no episode number — cannot look up credits")
        })?;
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
