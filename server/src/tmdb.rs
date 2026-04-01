use serde::Deserialize;
use std::path::Path;

const TMDB_BASE: &str = "https://api.themoviedb.org/3";
const TMDB_IMAGE_BASE: &str = "https://image.tmdb.org/t/p";

#[derive(Clone)]
pub struct TmdbClient {
    api_key: String,
    http: reqwest::Client,
}

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
}

#[derive(Debug, Clone)]
pub struct SeriesMetadata {
    pub tmdb_id: u64,
    pub name: String,
    pub overview: Option<String>,
    pub first_air_date: Option<String>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
}

impl TmdbClient {
    pub fn new(api_key: String) -> Self {
        TmdbClient {
            api_key,
            http: reqwest::Client::new(),
        }
    }

    /// Search TMDB for a TV series by name and return the best match
    pub async fn search_series(&self, query: &str) -> Result<Option<SeriesMetadata>, reqwest::Error> {
        let resp: SearchResponse = self
            .http
            .get(format!("{TMDB_BASE}/search/tv"))
            .query(&[("api_key", &self.api_key), ("query", &query.to_string())])
            .send()
            .await?
            .json()
            .await?;

        Ok(resp.results.into_iter().next().map(|r| SeriesMetadata {
            tmdb_id: r.id,
            name: r.name,
            overview: r.overview,
            first_air_date: r.first_air_date,
            poster_url: r.poster_path.map(|p| format!("{TMDB_IMAGE_BASE}/w500{p}")),
            backdrop_url: r.backdrop_path.map(|p| format!("{TMDB_IMAGE_BASE}/w1280{p}")),
        }))
    }

    /// Download a poster image and save it to the series directory
    pub async fn download_poster(&self, series_dir: &Path, poster_url: &str) -> Result<(), Box<dyn std::error::Error>> {
        let bytes = self.http.get(poster_url).send().await?.bytes().await?;
        let ext = if poster_url.ends_with(".png") { "png" } else { "jpg" };
        let dest = series_dir.join(format!("poster.{ext}"));
        tokio::fs::write(&dest, &bytes).await?;
        tracing::info!("Downloaded poster to {:?}", dest);
        Ok(())
    }

    /// Download a backdrop image and save it to the series directory
    pub async fn download_backdrop(
        &self,
        series_dir: &Path,
        backdrop_url: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let bytes = self.http.get(backdrop_url).send().await?.bytes().await?;
        let ext = if backdrop_url.ends_with(".png") { "png" } else { "jpg" };
        let dest = series_dir.join(format!("backdrop.{ext}"));
        tokio::fs::write(&dest, &bytes).await?;
        tracing::info!("Downloaded backdrop to {:?}", dest);
        Ok(())
    }
}

/// Fetch metadata for all series that don't already have art, download posters
pub async fn fetch_missing_art(
    client: &TmdbClient,
    media_root: &Path,
    series_titles: Vec<(String, bool)>, // (folder_name, has_art)
) -> usize {
    let mut downloaded = 0;

    for (title, has_art) in series_titles {
        if has_art {
            continue;
        }

        match client.search_series(&title).await {
            Ok(Some(meta)) => {
                if let Some(ref poster_url) = meta.poster_url {
                    let series_dir = media_root.join(&title);
                    if let Err(e) = client.download_poster(&series_dir, poster_url).await {
                        tracing::warn!("Failed to download poster for '{}': {}", title, e);
                    } else {
                        downloaded += 1;
                    }
                }
                // Also grab backdrop if available
                if let Some(ref backdrop_url) = meta.backdrop_url {
                    let series_dir = media_root.join(&title);
                    if let Err(e) = client.download_backdrop(&series_dir, backdrop_url).await {
                        tracing::warn!("Failed to download backdrop for '{}': {}", title, e);
                    }
                }
            }
            Ok(None) => {
                tracing::info!("No TMDB match found for '{}'", title);
            }
            Err(e) => {
                tracing::warn!("TMDB search failed for '{}': {}", title, e);
            }
        }
    }

    downloaded
}
