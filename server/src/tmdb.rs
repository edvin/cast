use regex::Regex;
use serde::Deserialize;
use std::path::Path;
use std::sync::LazyLock;

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

// --- Episode credits types ---

#[derive(Debug, Deserialize)]
struct TmdbCreditsResponse {
    cast: Option<Vec<TmdbCastMember>>,
    guest_stars: Option<Vec<TmdbCastMember>>,
}

#[derive(Debug, Deserialize)]
struct TmdbCastMember {
    id: u64,
    name: String,
    character: Option<String>,
    profile_path: Option<String>,
    order: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CastMember {
    pub id: u64,
    pub name: String,
    pub character: Option<String>,
    pub profile_url: Option<String>,
    pub order: u32,
    pub is_guest: bool,
}

// --- Person detail types ---

#[derive(Debug, Deserialize)]
struct TmdbPersonDetail {
    id: u64,
    name: String,
    biography: Option<String>,
    birthday: Option<String>,
    deathday: Option<String>,
    place_of_birth: Option<String>,
    profile_path: Option<String>,
    combined_credits: Option<TmdbCombinedCredits>,
}

#[derive(Debug, Deserialize)]
struct TmdbCombinedCredits {
    cast: Option<Vec<TmdbCreditRole>>,
}

#[derive(Debug, Deserialize)]
struct TmdbCreditRole {
    id: u64,
    media_type: Option<String>,
    title: Option<String>,       // for movies
    name: Option<String>,        // for TV
    character: Option<String>,
    poster_path: Option<String>,
    vote_average: Option<f64>,
    release_date: Option<String>,    // movies
    first_air_date: Option<String>,  // TV
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PersonDetail {
    pub id: u64,
    pub name: String,
    pub biography: Option<String>,
    pub birthday: Option<String>,
    pub deathday: Option<String>,
    pub place_of_birth: Option<String>,
    pub profile_url: Option<String>,
    pub known_for: Vec<CreditRole>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CreditRole {
    pub id: u64,
    pub title: String,
    pub character: Option<String>,
    pub media_type: String,
    pub poster_url: Option<String>,
    pub rating: Option<f64>,
    pub year: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EpisodeCredits {
    pub cast: Vec<CastMember>,
    pub guest_stars: Vec<CastMember>,
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

    /// Get cast and guest stars for a specific episode
    pub async fn get_episode_credits(
        &self,
        tmdb_id: u64,
        season_number: u32,
        episode_number: u32,
    ) -> Result<EpisodeCredits, reqwest::Error> {
        let resp = self
            .http
            .get(format!(
                "{TMDB_BASE}/tv/{tmdb_id}/season/{season_number}/episode/{episode_number}/credits"
            ))
            .query(&[("api_key", &self.api_key)])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(EpisodeCredits {
                cast: vec![],
                guest_stars: vec![],
            });
        }

        let credits: TmdbCreditsResponse = resp.json().await?;

        let cast = credits
            .cast
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, c)| CastMember {
                id: c.id,
                name: c.name,
                character: c.character,
                profile_url: c.profile_path.map(|p| format!("{TMDB_IMAGE_BASE}/w185{p}")),
                order: c.order.unwrap_or(i as u32),
                is_guest: false,
            })
            .collect();

        let guest_stars = credits
            .guest_stars
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, c)| CastMember {
                id: c.id,
                name: c.name,
                character: c.character,
                profile_url: c.profile_path.map(|p| format!("{TMDB_IMAGE_BASE}/w185{p}")),
                order: c.order.unwrap_or(i as u32),
                is_guest: true,
            })
            .collect();

        Ok(EpisodeCredits { cast, guest_stars })
    }

    /// Get person details with filmography
    pub async fn get_person_detail(&self, person_id: u64) -> Result<Option<PersonDetail>, reqwest::Error> {
        let resp = self
            .http
            .get(format!("{TMDB_BASE}/person/{person_id}"))
            .query(&[
                ("api_key", self.api_key.as_str()),
                ("append_to_response", "combined_credits"),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(None);
        }

        let detail: TmdbPersonDetail = resp.json().await?;

        let mut known_for: Vec<CreditRole> = detail
            .combined_credits
            .and_then(|c| c.cast)
            .unwrap_or_default()
            .into_iter()
            .map(|c| {
                let title = c.title.or(c.name).unwrap_or_default();
                let media_type = c.media_type.unwrap_or_else(|| "unknown".to_string());
                let year = c.release_date.or(c.first_air_date)
                    .and_then(|d| d.get(..4).map(|s| s.to_string()));
                CreditRole {
                    id: c.id,
                    title,
                    character: c.character,
                    media_type,
                    poster_url: c.poster_path.map(|p| format!("{TMDB_IMAGE_BASE}/w185{p}")),
                    rating: c.vote_average,
                    year,
                }
            })
            .collect();

        // Sort by rating descending (best known roles first)
        known_for.sort_by(|a, b| b.rating.partial_cmp(&a.rating).unwrap_or(std::cmp::Ordering::Equal));

        Ok(Some(PersonDetail {
            id: detail.id,
            name: detail.name,
            biography: detail.biography.filter(|b| !b.is_empty()),
            birthday: detail.birthday,
            deathday: detail.deathday,
            place_of_birth: detail.place_of_birth,
            profile_url: detail.profile_path.map(|p| format!("{TMDB_IMAGE_BASE}/w500{p}")),
            known_for,
        }))
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
        let dest = series_dir.join(format!(".poster.{ext}"));
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
        let dest = series_dir.join(format!(".backdrop.{ext}"));
        self.download_image(backdrop_url, &dest).await?;
        tracing::info!("Downloaded backdrop to {dest:?}");
        Ok(())
    }
}

// Regex patterns for cleaning folder names
static RE_YEAR_PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(\d{4}\)").unwrap());
static RE_YEAR_BRACKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\d{4}\]").unwrap());
static RE_YEAR_BARE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{4}\b").unwrap());
static RE_RESOLUTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(1080p|720p|4K|2160p|480p)\b").unwrap());
static RE_SOURCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(BluRay|WEB-DL|WEBRip|HDTV|BRRip|DVDRip)\b").unwrap());
static RE_ENCODING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(x264|x265|H[.\s]?264|H[.\s]?265|HEVC|AAC|DTS)\b").unwrap());
static RE_SEASON_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bS\d+\b").unwrap());
static RE_MULTI_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s{2,}").unwrap());

/// Clean a series folder name for TMDB search by removing year patterns,
/// resolution/source/encoding tags, and normalizing separators.
pub fn clean_search_query(folder_name: &str) -> String {
    let s = folder_name.replace(['.', '_'], " ");
    let s = RE_YEAR_PAREN.replace_all(&s, "");
    let s = RE_YEAR_BRACKET.replace_all(&s, "");
    let s = RE_RESOLUTION.replace_all(&s, "");
    let s = RE_SOURCE.replace_all(&s, "");
    let s = RE_ENCODING.replace_all(&s, "");
    let s = RE_SEASON_TAG.replace_all(&s, "");
    let s = RE_YEAR_BARE.replace_all(&s, "");
    let s = RE_MULTI_SPACE.replace_all(&s, " ");
    s.trim().to_string()
}

/// Fetch metadata for all series, download art, and store metadata in DB
pub async fn fetch_all_metadata(
    client: &TmdbClient,
    db: &crate::db::Database,
    media_root: &Path,
    series_list: Vec<(String, String, bool, bool, Option<u64>)>, // (series_id, folder_name, has_art, has_backdrop, tmdb_id_override)
) -> usize {
    let mut downloaded = 0;

    for (series_id, title, has_art, has_backdrop, tmdb_id_override) in series_list {
        // Check if we already have metadata, art, and backdrop
        if db.get_series_metadata(&series_id).is_some() && has_art && has_backdrop {
            continue;
        }

        // Resolve series info: use override ID, cleaned name search, or raw name fallback
        let search_result = if let Some(tmdb_id) = tmdb_id_override {
            tracing::info!("Using TMDB ID override {tmdb_id} for '{title}'");
            client.get_series_detail(tmdb_id).await
        } else {
            let cleaned = clean_search_query(&title);
            let result = if cleaned != title && !cleaned.is_empty() {
                tracing::debug!("Searching TMDB with cleaned name: '{cleaned}' (was '{title}')");
                client.search_series(&cleaned).await
            } else {
                client.search_series(&title).await
            };
            // Fall back to raw folder name if cleaned search found nothing
            match &result {
                Ok(None) if cleaned != title && !cleaned.is_empty() => {
                    tracing::debug!("Cleaned search found nothing, falling back to raw name: '{title}'");
                    client.search_series(&title).await
                }
                _ => result,
            }
        };

        match search_result {
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
                    let backdrop_path = media_root.join(&title).join(".backdrop.jpg");
                    let legacy_path = media_root.join(&title).join("backdrop.jpg");
                    if !backdrop_path.exists() && !legacy_path.exists() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_search_query_removes_year_and_resolution() {
        assert_eq!(clean_search_query("Breaking Bad (2008) 1080p"), "Breaking Bad");
    }

    #[test]
    fn clean_search_query_removes_dots_and_tags() {
        assert_eq!(clean_search_query("The.Wire.S01.720p.BluRay"), "The Wire");
    }

    #[test]
    fn clean_search_query_removes_brackets_and_encoding() {
        assert_eq!(clean_search_query("My Show [2020] WEB-DL x265"), "My Show");
    }

    #[test]
    fn clean_search_query_preserves_simple_name() {
        assert_eq!(clean_search_query("Simple Name"), "Simple Name");
    }

    #[test]
    fn clean_search_query_removes_multiple_tags() {
        assert_eq!(clean_search_query("Some.Show.2019.1080p.WEB-DL.H.265.AAC"), "Some Show");
    }

    #[test]
    fn clean_search_query_handles_underscores() {
        assert_eq!(clean_search_query("My_Show_2020_HDTV"), "My Show");
    }
}
