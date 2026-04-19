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

// --- Movie types ---

#[derive(Debug, Deserialize)]
struct MovieSearchResponse {
    results: Vec<MovieResult>,
}

#[derive(Debug, Deserialize)]
struct MovieResult {
    id: u64,
    title: String,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    overview: Option<String>,
    release_date: Option<String>,
    vote_average: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct TmdbMovieDetail {
    id: u64,
    title: String,
    overview: Option<String>,
    release_date: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    vote_average: Option<f64>,
    runtime: Option<u32>,
    tagline: Option<String>,
    genres: Option<Vec<Genre>>,
}

#[derive(Debug, Clone)]
pub struct MovieInfo {
    pub tmdb_id: u64,
    pub title: String,
    pub overview: Option<String>,
    pub release_date: Option<String>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub runtime_minutes: Option<u32>,
    pub tagline: Option<String>,
    pub genres: Vec<String>,
    pub rating: Option<f64>,
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
    title: Option<String>, // for movies
    name: Option<String>,  // for TV
    character: Option<String>,
    poster_path: Option<String>,
    vote_average: Option<f64>,
    release_date: Option<String>,   // movies
    first_air_date: Option<String>, // TV
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

    /// Search TMDB for a movie by title (optional year improves the match).
    pub async fn search_movie(&self, query: &str, year: Option<&str>) -> Result<Option<MovieInfo>, reqwest::Error> {
        let mut params: Vec<(&str, &str)> = vec![("api_key", &self.api_key), ("query", query)];
        if let Some(y) = year {
            params.push(("year", y));
        }
        let resp: MovieSearchResponse = self
            .http
            .get(format!("{TMDB_BASE}/search/movie"))
            .query(&params)
            .send()
            .await?
            .json()
            .await?;

        match resp.results.into_iter().next() {
            Some(r) => {
                // Fetch full details for runtime, tagline, genres
                match self.get_movie_detail(r.id).await {
                    Ok(Some(detail)) => Ok(Some(detail)),
                    _ => Ok(Some(MovieInfo {
                        tmdb_id: r.id,
                        title: r.title,
                        overview: r.overview,
                        release_date: r.release_date,
                        poster_url: r.poster_path.map(|p| format!("{TMDB_IMAGE_BASE}/w500{p}")),
                        backdrop_url: r.backdrop_path.map(|p| format!("{TMDB_IMAGE_BASE}/w1280{p}")),
                        runtime_minutes: None,
                        tagline: None,
                        genres: vec![],
                        rating: r.vote_average,
                    })),
                }
            }
            None => Ok(None),
        }
    }

    /// Fetch full movie detail by TMDB id.
    pub async fn get_movie_detail(&self, tmdb_id: u64) -> Result<Option<MovieInfo>, reqwest::Error> {
        let resp = self
            .http
            .get(format!("{TMDB_BASE}/movie/{tmdb_id}"))
            .query(&[("api_key", &self.api_key)])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let detail: TmdbMovieDetail = resp.json().await?;
        let genres: Vec<String> = detail.genres.unwrap_or_default().into_iter().map(|g| g.name).collect();
        Ok(Some(MovieInfo {
            tmdb_id: detail.id,
            title: detail.title,
            overview: detail.overview,
            release_date: detail.release_date,
            poster_url: detail.poster_path.map(|p| format!("{TMDB_IMAGE_BASE}/w500{p}")),
            backdrop_url: detail.backdrop_path.map(|p| format!("{TMDB_IMAGE_BASE}/w1280{p}")),
            runtime_minutes: detail.runtime,
            tagline: detail.tagline.filter(|t| !t.is_empty()),
            genres,
            rating: detail.vote_average,
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
                let year = c
                    .release_date
                    .or(c.first_air_date)
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

        // Sort by rating descending (best known roles first). Ratings are optional and can be
        // NaN from malformed TMDB payloads, so normalise to a total-orderable f64 via total_cmp.
        known_for.sort_by(|a, b| {
            let ar = a.rating.filter(|r| r.is_finite()).unwrap_or(f64::NEG_INFINITY);
            let br = b.rating.filter(|r| r.is_finite()).unwrap_or(f64::NEG_INFINITY);
            br.total_cmp(&ar)
        });

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

    /// Fetch the raw bytes + content type of an image. Used when we're storing
    /// artwork in the database instead of on the filesystem.
    pub async fn fetch_image_bytes(&self, url: &str) -> Result<(String, Vec<u8>), Box<dyn std::error::Error>> {
        let resp = self.http.get(url).send().await?;
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/jpeg")
            .to_string();
        let bytes = resp.bytes().await?.to_vec();
        Ok((ct, bytes))
    }

    /// Download a poster image and save it to the series directory
    pub async fn download_poster(&self, series_dir: &Path, poster_url: &str) -> Result<(), Box<dyn std::error::Error>> {
        let ext = image_extension(poster_url);
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
        let ext = image_extension(backdrop_url);
        let dest = series_dir.join(format!(".backdrop.{ext}"));
        self.download_image(backdrop_url, &dest).await?;
        tracing::info!("Downloaded backdrop to {dest:?}");
        Ok(())
    }
}

/// Extract the file extension from an image URL, ignoring query strings and fragments.
/// Defaults to "jpg" if no recognised extension is found.
fn image_extension(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    if path.to_ascii_lowercase().ends_with(".png") {
        "png"
    } else {
        "jpg"
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

/// One movie to fetch metadata for. Passed in by value from the scan so we don't
/// hold a library read lock across the network round-trip.
pub struct MovieFetchEntry {
    pub movie_id: String,
    pub title: String,
    pub year: Option<String>,
    /// Relative path from media_root to the video file
    pub video_path: std::path::PathBuf,
    pub has_art: bool,
    pub has_backdrop: bool,
    pub tmdb_id_override: Option<u64>,
}

/// Fetch TMDB metadata + art for a list of movies. Mirrors fetch_all_metadata but
/// targets the /movie/... endpoints.
pub async fn fetch_all_movies_metadata(
    client: &TmdbClient,
    db: &crate::db::Database,
    _media_root: &Path,
    movies: Vec<MovieFetchEntry>,
    log: impl Fn(&str),
) -> usize {
    let mut downloaded = 0;
    let total = movies.len();
    let mut skipped_abandoned = 0u32;
    for (i, entry) in movies.into_iter().enumerate() {
        let has_db_art = entry.has_art || db.has_artwork(&entry.movie_id, "art");
        let has_db_backdrop = entry.has_backdrop || db.has_artwork(&entry.movie_id, "backdrop");
        if db.get_movie_metadata(&entry.movie_id).is_some() && has_db_art && has_db_backdrop {
            continue;
        }
        if entry.tmdb_id_override.is_none() && db.is_tmdb_lookup_abandoned(&entry.movie_id) {
            skipped_abandoned += 1;
            continue;
        }

        let msg = format!("TMDB movie [{}/{}]: {}", i + 1, total, entry.title);
        tracing::info!("{msg}");
        log(&msg);

        let search_result = if let Some(tmdb_id) = entry.tmdb_id_override {
            let m = format!("Using TMDB movie ID override {tmdb_id} for '{}'", entry.title);
            tracing::info!("{m}");
            log(&m);
            client.get_movie_detail(tmdb_id).await
        } else {
            client.search_movie(&entry.title, entry.year.as_deref()).await
        };

        match search_result {
            Ok(Some(info)) => {
                db.clear_tmdb_failure(&entry.movie_id);

                let meta = crate::db::MovieMetadata {
                    movie_id: entry.movie_id.clone(),
                    tmdb_id: Some(info.tmdb_id),
                    title: Some(info.title.clone()),
                    overview: info.overview.clone(),
                    release_date: info.release_date.clone(),
                    runtime_minutes: info.runtime_minutes,
                    genres: if info.genres.is_empty() {
                        None
                    } else {
                        Some(info.genres.join(", "))
                    },
                    rating: info.rating,
                    tagline: info.tagline.clone(),
                };
                if let Err(e) = db.save_movie_metadata(&meta) {
                    let m = format!("Failed to save movie metadata for '{}': {e}", entry.title);
                    tracing::warn!("{m}");
                    log(&m);
                }

                if !has_db_art {
                    if let Some(ref url) = info.poster_url {
                        match client.fetch_image_bytes(url).await {
                            Ok((ct, bytes)) => {
                                if let Err(e) = db.save_artwork(&entry.movie_id, "art", &ct, &bytes) {
                                    let m = format!("Failed to save poster for '{}': {e}", entry.title);
                                    tracing::warn!("{m}");
                                    log(&m);
                                } else {
                                    downloaded += 1;
                                }
                            }
                            Err(e) => {
                                let m = format!("Failed to fetch poster for '{}': {e}", entry.title);
                                tracing::warn!("{m}");
                                log(&m);
                            }
                        }
                    }
                }

                if !has_db_backdrop {
                    if let Some(ref url) = info.backdrop_url {
                        match client.fetch_image_bytes(url).await {
                            Ok((ct, bytes)) => {
                                if let Err(e) = db.save_artwork(&entry.movie_id, "backdrop", &ct, &bytes) {
                                    let m = format!("Failed to save backdrop for '{}': {e}", entry.title);
                                    tracing::warn!("{m}");
                                    log(&m);
                                }
                            }
                            Err(e) => {
                                let m = format!("Failed to fetch backdrop for '{}': {e}", entry.title);
                                tracing::warn!("{m}");
                                log(&m);
                            }
                        }
                    }
                }
            }
            Ok(None) => {
                let state = db.record_tmdb_failure(&entry.movie_id, "movie", "no TMDB match", MAX_TMDB_LOOKUP_ATTEMPTS);
                if state.given_up {
                    let m = format!(
                        "Giving up on '{}' after {} TMDB lookup attempts — add tmdb.txt with the movie ID or click Retry",
                        entry.title, state.attempts
                    );
                    tracing::info!("{m}");
                    log(&m);
                } else {
                    let m = format!(
                        "No TMDB match for '{}' (attempt {}/{MAX_TMDB_LOOKUP_ATTEMPTS})",
                        entry.title, state.attempts
                    );
                    tracing::info!("{m}");
                    log(&m);
                }
            }
            Err(e) => {
                let state = db.record_tmdb_failure(&entry.movie_id, "movie", &format!("{e}"), MAX_TMDB_LOOKUP_ATTEMPTS);
                let m = format!(
                    "TMDB movie search failed for '{}' (attempt {}/{MAX_TMDB_LOOKUP_ATTEMPTS}): {e}",
                    entry.title, state.attempts
                );
                tracing::warn!("{m}");
                log(&m);
            }
        }
    }
    if skipped_abandoned > 0 {
        log(&format!(
            "Skipped {skipped_abandoned} movie(s) previously given up on — use Retry Metadata to try again"
        ));
    }
    downloaded
}

/// Max lookup attempts before we flag a series/movie as "given up" and stop
/// re-querying TMDB for it every rescan cycle. User can clear the flag via
/// POST /api/metadata/retry.
pub const MAX_TMDB_LOOKUP_ATTEMPTS: i64 = 3;

/// Fetch metadata for all series, download art, and store metadata in DB.
/// `log` is invoked for per-series progress so it shows up in the desktop UI log.
pub async fn fetch_all_metadata(
    client: &TmdbClient,
    db: &crate::db::Database,
    _media_root: &Path,
    series_list: Vec<(String, String, bool, bool, Option<u64>)>, // (series_id, folder_name, has_art, has_backdrop, tmdb_id_override)
    log: impl Fn(&str),
) -> usize {
    let mut downloaded = 0;

    let total = series_list.len();
    let mut skipped_abandoned = 0u32;
    for (i, (series_id, title, has_art, has_backdrop, tmdb_id_override)) in series_list.into_iter().enumerate() {
        let has_db_art = has_art || db.has_artwork(&series_id, "art");
        let has_db_backdrop = has_backdrop || db.has_artwork(&series_id, "backdrop");
        // Check if we already have metadata, art, and backdrop
        if db.get_series_metadata(&series_id).is_some() && has_db_art && has_db_backdrop {
            continue;
        }
        // Skip items the user has given up on (unless they've added a tmdb.txt override,
        // which is an explicit signal to try again).
        if tmdb_id_override.is_none() && db.is_tmdb_lookup_abandoned(&series_id) {
            skipped_abandoned += 1;
            continue;
        }

        let progress_msg = format!("TMDB [{}/{}]: {}", i + 1, total, title);
        tracing::info!("{progress_msg}");
        log(&progress_msg);

        // Resolve series info: use override ID, cleaned name search, or raw name fallback
        let search_result = if let Some(tmdb_id) = tmdb_id_override {
            let msg = format!("Using TMDB ID override {tmdb_id} for '{title}'");
            tracing::info!("{msg}");
            log(&msg);
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
                db.clear_tmdb_failure(&series_id);

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
                    let msg = format!("Failed to save metadata for '{title}': {e}");
                    tracing::warn!("{msg}");
                    log(&msg);
                }

                // Poster: save into DB as BLOB (no more littering the filesystem)
                if !has_db_art {
                    if let Some(ref poster_url) = info.poster_url {
                        match client.fetch_image_bytes(poster_url).await {
                            Ok((ct, bytes)) => {
                                if let Err(e) = db.save_artwork(&series_id, "art", &ct, &bytes) {
                                    let msg = format!("Failed to save poster for '{title}': {e}");
                                    tracing::warn!("{msg}");
                                    log(&msg);
                                } else {
                                    downloaded += 1;
                                }
                            }
                            Err(e) => {
                                let msg = format!("Failed to fetch poster for '{title}': {e}");
                                tracing::warn!("{msg}");
                                log(&msg);
                            }
                        }
                    }
                }

                // Backdrop: same — save into DB
                if !has_db_backdrop {
                    if let Some(ref backdrop_url) = info.backdrop_url {
                        match client.fetch_image_bytes(backdrop_url).await {
                            Ok((ct, bytes)) => {
                                if let Err(e) = db.save_artwork(&series_id, "backdrop", &ct, &bytes) {
                                    let msg = format!("Failed to save backdrop for '{title}': {e}");
                                    tracing::warn!("{msg}");
                                    log(&msg);
                                }
                            }
                            Err(e) => {
                                let msg = format!("Failed to fetch backdrop for '{title}': {e}");
                                tracing::warn!("{msg}");
                                log(&msg);
                            }
                        }
                    }
                }

                // Fetch episode metadata for all seasons
                for season_num in &info.seasons {
                    match client.get_season_episodes(info.tmdb_id, *season_num).await {
                        Ok(episodes) => {
                            for ep in episodes {
                                let ep_meta = crate::db::EpisodeMetadata {
                                    episode_id: String::new(),
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
                                    let msg = format!("Failed to save episode metadata: {e}");
                                    tracing::warn!("{msg}");
                                    log(&msg);
                                }
                            }
                        }
                        Err(e) => {
                            let msg = format!("Failed to fetch season {season_num} for '{title}': {e}");
                            tracing::warn!("{msg}");
                            log(&msg);
                        }
                    }
                }

                tracing::info!("Fetched metadata for '{title}' (TMDB ID: {})", info.tmdb_id);
            }
            Ok(None) => {
                let reason = "no TMDB match";
                let state = db.record_tmdb_failure(&series_id, "series", reason, MAX_TMDB_LOOKUP_ATTEMPTS);
                if state.given_up {
                    let msg = format!("Giving up on '{title}' after {} TMDB lookup attempts — add tmdb.txt with the series ID or click Retry to try again", state.attempts);
                    tracing::info!("{msg}");
                    log(&msg);
                } else {
                    let msg = format!(
                        "No TMDB match for '{title}' (attempt {}/{MAX_TMDB_LOOKUP_ATTEMPTS})",
                        state.attempts
                    );
                    tracing::info!("{msg}");
                    log(&msg);
                }
            }
            Err(e) => {
                let reason = format!("{e}");
                let state = db.record_tmdb_failure(&series_id, "series", &reason, MAX_TMDB_LOOKUP_ATTEMPTS);
                let msg = format!(
                    "TMDB search failed for '{title}' (attempt {}/{MAX_TMDB_LOOKUP_ATTEMPTS}): {e}",
                    state.attempts
                );
                tracing::warn!("{msg}");
                log(&msg);
            }
        }
    }

    if skipped_abandoned > 0 {
        log(&format!(
            "Skipped {skipped_abandoned} series previously given up on — use Retry Metadata to try again"
        ));
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
