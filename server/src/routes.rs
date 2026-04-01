use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

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
        .route("/api/series/{series_id}/backdrop", get(get_series_backdrop))
        .route("/api/metadata/fetch", post(fetch_metadata))
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
    /// Summary of watch state for this series
    watched_count: usize,
    total_count: usize,
}

#[derive(Serialize)]
struct SeriesDetail {
    id: String,
    title: String,
    has_art: bool,
    has_backdrop: bool,
    episodes: Vec<EpisodeItem>,
}

#[derive(Serialize)]
struct EpisodeItem {
    id: String,
    title: String,
    index: usize,
    size_bytes: u64,
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

#[derive(Deserialize)]
struct ProgressUpdate {
    position_secs: f64,
    duration_secs: f64,
}

// --- Handlers ---

async fn list_series(State(state): State<Arc<AppState>>) -> Json<Vec<SeriesListItem>> {
    let lib = state.library.read().await;
    let mut result: Vec<SeriesListItem> = lib
        .series
        .values()
        .map(|s| {
            let episode_ids: Vec<String> = s.episodes.iter().map(|e| e.id.clone()).collect();
            let progress = state.db.get_series_progress(&episode_ids);
            let watched_count = progress.iter().filter(|p| p.completed).count();

            SeriesListItem {
                id: s.id.clone(),
                title: s.title.clone(),
                episode_count: s.episodes.len(),
                has_art: s.art.is_some(),
                has_backdrop: s.backdrop.is_some(),
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
) -> Result<Json<SeriesDetail>, StatusCode> {
    let lib = state.library.read().await;
    let series = lib.find_series(&series_id).ok_or(StatusCode::NOT_FOUND)?;

    let episodes: Vec<EpisodeItem> = series
        .episodes
        .iter()
        .map(|ep| {
            let progress = state.db.get_progress(&ep.id).map(|p| EpisodeProgress {
                position_secs: p.position_secs,
                duration_secs: p.duration_secs,
                completed: p.completed,
            });

            EpisodeItem {
                id: ep.id.clone(),
                title: ep.title.clone(),
                index: ep.index,
                size_bytes: ep.size_bytes,
                progress,
            }
        })
        .collect();

    Ok(Json(SeriesDetail {
        id: series.id.clone(),
        title: series.title.clone(),
        has_art: series.art.is_some(),
        has_backdrop: series.backdrop.is_some(),
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
) -> Result<Json<NextEpisodeResponse>, StatusCode> {
    let lib = state.library.read().await;
    let series = lib.find_series(&series_id).ok_or(StatusCode::NOT_FOUND)?;

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
            return Ok(Json(NextEpisodeResponse {
                episode: Some(EpisodeItem {
                    id: ep.id.clone(),
                    title: ep.title.clone(),
                    index: ep.index,
                    size_bytes: ep.size_bytes,
                    progress: Some(EpisodeProgress {
                        position_secs: current.position_secs,
                        duration_secs: current.duration_secs,
                        completed: false,
                    }),
                }),
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
                episode: Some(EpisodeItem {
                    id: next_ep.id.clone(),
                    title: next_ep.title.clone(),
                    index: next_ep.index,
                    size_bytes: next_ep.size_bytes,
                    progress: None,
                }),
                reason: "next".to_string(),
            }));
        } else {
            // All episodes completed
            return Ok(Json(NextEpisodeResponse {
                episode: None,
                reason: "all_watched".to_string(),
            }));
        }
    }

    // Nothing watched — start from the beginning
    let first = &series.episodes[0];
    Ok(Json(NextEpisodeResponse {
        episode: Some(EpisodeItem {
            id: first.id.clone(),
            title: first.title.clone(),
            index: first.index,
            size_bytes: first.size_bytes,
            progress: None,
        }),
        reason: "first".to_string(),
    }))
}

async fn get_series_art(
    State(state): State<Arc<AppState>>,
    Path(series_id): Path<String>,
) -> Result<Response, StatusCode> {
    let lib = state.library.read().await;
    let series = lib.find_series(&series_id).ok_or(StatusCode::NOT_FOUND)?;
    let art_rel = series.art.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let art_path = state.media_path.join(art_rel);

    if !art_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let content_type = mime_guess::from_path(&art_path).first_or_octet_stream().to_string();

    let data = tokio::fs::read(&art_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(([(header::CONTENT_TYPE, content_type)], data).into_response())
}

/// Stream a video file with byte-range support for seeking
async fn stream_episode(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let lib = state.library.read().await;
    let (_series, episode) = lib.find_episode(&episode_id).ok_or(StatusCode::NOT_FOUND)?;
    let file_path = state.media_path.join(&episode.path);
    let file_size = episode.size_bytes;

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
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
) -> Result<Json<Option<EpisodeProgress>>, StatusCode> {
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
) -> Result<StatusCode, StatusCode> {
    state
        .db
        .update_progress(&episode_id, body.position_secs, body.duration_secs)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::OK)
}

async fn get_all_progress(State(state): State<Arc<AppState>>) -> Json<Vec<crate::db::WatchProgress>> {
    Json(state.db.get_all_progress())
}

async fn get_series_backdrop(
    State(state): State<Arc<AppState>>,
    Path(series_id): Path<String>,
) -> Result<Response, StatusCode> {
    let lib = state.library.read().await;
    let series = lib.find_series(&series_id).ok_or(StatusCode::NOT_FOUND)?;
    let backdrop_rel = series.backdrop.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let backdrop_path = state.media_path.join(backdrop_rel);

    if !backdrop_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let content_type = mime_guess::from_path(&backdrop_path)
        .first_or_octet_stream()
        .to_string();

    let data = tokio::fs::read(&backdrop_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(([(header::CONTENT_TYPE, content_type)], data).into_response())
}

#[derive(Serialize)]
struct FetchMetadataResponse {
    downloaded: usize,
    message: String,
}

/// Trigger TMDB metadata/art fetch for series missing artwork
async fn fetch_metadata(State(state): State<Arc<AppState>>) -> Result<Json<FetchMetadataResponse>, StatusCode> {
    let client = state.tmdb.as_ref().ok_or_else(|| {
        tracing::warn!("TMDB fetch requested but no API key configured");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    let series_info: Vec<(String, bool)> = {
        let lib = state.library.read().await;
        lib.series
            .values()
            .map(|s| (s.title.clone(), s.art.is_some()))
            .collect()
    };

    let missing = series_info.iter().filter(|(_, has)| !has).count();
    let downloaded = crate::tmdb::fetch_missing_art(client, &state.media_path, series_info).await;

    // Rescan library to pick up new art files
    if downloaded > 0 {
        match crate::library::Library::scan(&state.media_path) {
            Ok(lib) => *state.library.write().await = lib,
            Err(e) => tracing::warn!("Rescan after metadata fetch failed: {e}"),
        }
    }

    Ok(Json(FetchMetadataResponse {
        downloaded,
        message: format!("{downloaded} of {missing} series updated"),
    }))
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
        assert_eq!(episodes[0]["title"], "S01E01");
        assert_eq!(episodes[1]["title"], "S01E02");
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
}
