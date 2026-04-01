use serde::Deserialize;
use std::path::Path;

const TMDB_BASE: &str = "https://api.themoviedb.org/3";
const TMDB_IMAGE_BASE: &str = "https://image.tmdb.org/t/p";

#[derive(Clone)]
pub struct TmdbClient {
    api_key: String,
    http: reqwest::Client,
}

// --- Search types ---

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<TvResult>,
}

#[derive(Debug, Deserialize)]
struct TvResult {
    id: u64,
    name: String,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    overview: Option<String>,
    first_air_date: Option<String>,
    vote_average: Option<f64>,
}

// --- TV detail types ---

#[derive(Debug, Deserialize)]
struct TvDetail {
    id: u64,
    name: String,
    overview: Option<String>,
    first_air_date: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    vote_average: Option<f64>,
    genres: Option<Vec<Genre>>,
    seasons: Option<Vec<SeasonSummary>>,
}

#[derive(Debug, Deserialize)]
struct Genre {
    name: String,
}

#[derive(Debug, Deserialize)]
struct SeasonSummary {
    season_number: u32,
    episode_count: u32,
}

// --- Season detail types ---

#[derive(Debug, Deserialize)]
struct SeasonDetail {
    episodes: Option<Vec<TmdbEpisode>>,
}

#[derive(Debug, Deserialize)]
struct TmdbEpisode {
    id: u64,
    episode_number: u32,
    name: Option<String>,
    overview: Option<String>,
    air_date: Option<String>,
    runtime: Option<u32>,
    still_path: Option<String>,
}

// --- Public result types ---

#[derive(Debug, Clone)]
pub struct SeriesInfo {
    pub tmdb_id: u64,
    pub name: String,
    pub overview: Option<String>,
    pub first_air_date: Option<String>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub genres: Vec<String>,
    pub rating: Option<f64>,
    pub seasons: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct EpisodeInfo {
    pub tmdb_episode_id: u64,
    pub season_number: u32,
    pub episode_number: u32,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub runtime_minutes: Option<u32>,
    pub still_url: Option<String>,
}

impl TmdbClient {
    pub fn new(api_key: String) -> Self {
        TmdbClient {
            api_key,
            http: reqwest::Client::new(),
        }
    }

    /// Search TMDB for a TV series by name and return the best match
    pub async fn search_series(&self, query: &str) -> Result<Option<SeriesInfo>, reqwest::Error> {
        let resp: SearchResponse = self
            .http
            .get(format!("{TMDB_BASE}/search/tv"))
            .query(&[("api_key", &self.api_key), ("query", &query.to_string())])
            .send()
            .await?
            .json()
            .await?;

        match resp.results.into_iter().next() {
            Some(r) => {
                // Fetch full details to get genres and season list
                match self.get_series_detail(r.id).await {
                    Ok(Some(detail)) => Ok(Some(detail)),
                    _ => Ok(Some(SeriesInfo {
                        tmdb_id: r.id,
                        name: r.name,
                        overview: r.overview,
                        first_air_date: r.first_air_date,
                        poster_url: r.poster_path.map(|p| format!("{TMDB_IMAGE_BASE}/w500{p}")),
                        backdrop_url: r.backdrop_path.map(|p| format!("{TMDB_IMAGE_BASE}/w1280{p}")),
                        genres: vec![],
                        rating: r.vote_average,
                        seasons: vec![],
                    })),
                }
            }
            None => Ok(None),
        }
    }

    /// Get full series details including genres and season list
    pub async fn get_series_detail(&self, tmdb_id: u64) -> Result<Option<SeriesInfo>, reqwest::Error> {
        let resp = self
            .http
            .get(format!("{TMDB_BASE}/tv/{tmdb_id}"))
            .query(&[("api_key", &self.api_key)])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(None);
        }

        let detail: TvDetail = resp.json().await?;
        let genres: Vec<String> = detail.genres.unwrap_or_default().into_iter().map(|g| g.name).collect();
        let seasons: Vec<u32> = detail
            .seasons
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.season_number > 0 && s.episode_count > 0)
            .map(|s| s.season_number)
            .collect();

        Ok(Some(SeriesInfo {
            tmdb_id: detail.id,
            name: detail.name,
            overview: detail.overview,
            first_air_date: detail.first_air_date,
            poster_url: detail.poster_path.map(|p| format!("{TMDB_IMAGE_BASE}/w500{p}")),
            backdrop_url: detail.backdrop_path.map(|p| format!("{TMDB_IMAGE_BASE}/w1280{p}")),
            genres,
            rating: detail.vote_average,
            seasons,
        }))
    }

    /// Get episode details for a specific season
    pub async fn get_season_episodes(
        &self,
        tmdb_id: u64,
        season_number: u32,
    ) -> Result<Vec<EpisodeInfo>, reqwest::Error> {
        let resp = self
            .http
            .get(format!("{TMDB_BASE}/tv/{tmdb_id}/season/{season_number}"))
            .query(&[("api_key", &self.api_key)])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let detail: SeasonDetail = resp.json().await?;
        Ok(detail
            .episodes
            .unwrap_or_default()
            .into_iter()
            .map(|ep| EpisodeInfo {
                tmdb_episode_id: ep.id,
                season_number,
                episode_number: ep.episode_number,
                title: ep.name,
                overview: ep.overview,
                air_date: ep.air_date,
                runtime_minutes: ep.runtime,
                still_url: ep.still_path.map(|p| format!("{TMDB_IMAGE_BASE}/w300{p}")),
            })
            .collect())
    }

    /// Download an image from a URL and save it to a path
    pub async fn download_image(&self, url: &str, dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let bytes = self.http.get(url).send().await?.bytes().await?;
        tokio::fs::write(dest, &bytes).await?;
        Ok(())
    }

    /// Download a poster image and save it to the series directory
    pub async fn download_poster(&self, series_dir: &Path, poster_url: &str) -> Result<(), Box<dyn std::error::Error>> {
        let ext = if poster_url.ends_with(".png") { "png" } else { "jpg" };
        let dest = series_dir.join(format!("poster.{ext}"));
        self.download_image(poster_url, &dest).await?;
        tracing::info!("Downloaded poster to {dest:?}");
        Ok(())
    }

    /// Download a backdrop image and save it to the series directory
    pub async fn download_backdrop(
        &self,
        series_dir: &Path,
        backdrop_url: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let ext = if backdrop_url.ends_with(".png") { "png" } else { "jpg" };
        let dest = series_dir.join(format!("backdrop.{ext}"));
        self.download_image(backdrop_url, &dest).await?;
        tracing::info!("Downloaded backdrop to {dest:?}");
        Ok(())
    }
}

/// Fetch metadata for all series, download art, and store metadata in DB
pub async fn fetch_all_metadata(
    client: &TmdbClient,
    db: &crate::db::Database,
    media_root: &Path,
    series_list: Vec<(String, String, bool)>, // (series_id, folder_name, has_art)
) -> usize {
    let mut downloaded = 0;

    for (series_id, title, has_art) in series_list {
        // Check if we already have metadata for this series
        if db.get_series_metadata(&series_id).is_some() && has_art {
            continue;
        }

        match client.search_series(&title).await {
            Ok(Some(info)) => {
                // Save series metadata to DB
                let meta = crate::db::SeriesMetadata {
                    series_id: series_id.clone(),
                    tmdb_id: Some(info.tmdb_id),
                    title: Some(info.name.clone()),
                    overview: info.overview.clone(),
                    first_air_date: info.first_air_date.clone(),
                    genres: if info.genres.is_empty() {
                        None
                    } else {
                        Some(info.genres.join(", "))
                    },
                    rating: info.rating,
                };
                if let Err(e) = db.save_series_metadata(&meta) {
                    tracing::warn!("Failed to save metadata for '{title}': {e}");
                }

                // Download poster if missing
                if !has_art {
                    if let Some(ref poster_url) = info.poster_url {
                        let series_dir = media_root.join(&title);
                        if let Err(e) = client.download_poster(&series_dir, poster_url).await {
                            tracing::warn!("Failed to download poster for '{title}': {e}");
                        } else {
                            downloaded += 1;
                        }
                    }
                }

                // Download backdrop if missing
                if let Some(ref backdrop_url) = info.backdrop_url {
                    let backdrop_path = media_root.join(&title).join("backdrop.jpg");
                    if !backdrop_path.exists() {
                        if let Err(e) = client.download_backdrop(&media_root.join(&title), backdrop_url).await {
                            tracing::warn!("Failed to download backdrop for '{title}': {e}");
                        }
                    }
                }

                // Fetch episode metadata for all seasons
                for season_num in &info.seasons {
                    match client.get_season_episodes(info.tmdb_id, *season_num).await {
                        Ok(episodes) => {
                            for ep in episodes {
                                let ep_meta = crate::db::EpisodeMetadata {
                                    episode_id: String::new(), // Will be matched later
                                    series_id: series_id.clone(),
                                    tmdb_episode_id: Some(ep.tmdb_episode_id),
                                    season_number: Some(ep.season_number),
                                    episode_number: Some(ep.episode_number),
                                    title: ep.title,
                                    overview: ep.overview,
                                    air_date: ep.air_date,
                                    runtime_minutes: ep.runtime_minutes,
                                    still_url: ep.still_url,
                                };
                                if let Err(e) = db.save_episode_metadata(&ep_meta) {
                                    tracing::warn!("Failed to save episode metadata: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to fetch season {season_num} for '{title}': {e}");
                        }
                    }
                }

                tracing::info!("Fetched metadata for '{title}' (TMDB ID: {})", info.tmdb_id);
            }
            Ok(None) => {
                tracing::info!("No TMDB match found for '{title}'");
            }
            Err(e) => {
                tracing::warn!("TMDB search failed for '{title}': {e}");
            }
        }
    }

    downloaded
}
