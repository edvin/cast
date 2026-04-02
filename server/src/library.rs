use regex::Regex;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;
use uuid::Uuid;

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "mkv", "avi", "webm"];
const ART_NAMES: &[&str] = &[
    ".poster.jpg",
    ".poster.png",
    // Legacy names (for backward compatibility)
    "poster.jpg",
    "poster.png",
    "folder.jpg",
    "folder.png",
    "cover.jpg",
    "cover.png",
];
const BACKDROP_NAMES: &[&str] = &[
    ".backdrop.jpg",
    ".backdrop.png",
    // Legacy names
    "backdrop.jpg",
    "backdrop.png",
    "fanart.jpg",
    "fanart.png",
];

// UUID v5 namespace for generating stable IDs from paths
const NAMESPACE: Uuid = Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
]);

#[derive(Debug, Clone, Serialize)]
pub struct Library {
    /// series_id -> Series
    pub series: BTreeMap<String, Series>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Series {
    pub id: String,
    pub title: String,
    /// Relative path from media root to the series folder
    pub path: String,
    /// Art file relative path (if found)
    pub art: Option<String>,
    /// Backdrop/fanart relative path (if found)
    pub backdrop: Option<String>,
    /// Manual TMDB ID override from tmdb.txt in series folder
    pub tmdb_id_override: Option<u64>,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Episode {
    pub id: String,
    pub title: String,
    /// Relative path from media root
    pub path: String,
    pub filename: String,
    /// File size in bytes
    pub size_bytes: u64,
    /// Sort index (derived from filename ordering)
    pub index: usize,
    pub season_number: Option<u32>,
    pub episode_number: Option<u32>,
    /// External subtitle files found (relative paths from media root)
    pub subtitles: Vec<SubtitleFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubtitleFile {
    pub language: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct ParsedEpisodeInfo {
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub title: String,
}

fn stable_id(path: &str) -> String {
    Uuid::new_v5(&NAMESPACE, path.as_bytes()).to_string().replace('-', "")[..12].to_string()
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Clean up a raw title extracted from a filename: replace dots/underscores with spaces,
/// strip leading separator characters, and trim whitespace.
fn clean_title(raw: &str) -> String {
    let s = raw.replace(['.', '_'], " ");
    let s = s.trim();
    // Strip a leading " - " or "- " or " -" separator
    let s = s.strip_prefix("- ").or_else(|| s.strip_prefix('-')).unwrap_or(s);
    let s = s.trim();
    // Strip common scene-release tags (resolution, codec, source, group)
    strip_release_tags(s)
}

/// Remove common scene-release tags like 720p, 1080p, WEB, H264, x265, group names, etc.
fn strip_release_tags(s: &str) -> String {
    static RE_TAGS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(720p|1080p|2160p|4k|web|webrip|web-dl|hdtv|bluray|bdrip|dvdrip|h\.?264|h\.?265|x\.?264|x\.?265|aac|hevc|10bit|hdr|remux)\b").unwrap()
    });
    // Strip from the first recognized tag onward
    if let Some(m) = RE_TAGS.find(s) {
        let before = s[..m.start()].trim();
        // Also strip trailing dash and group name (e.g. "-SYLIX")
        let before = before.trim_end_matches(|c: char| c == '-' || c == '.' || c == ' ');
        return before.to_string(); // may be empty — caller handles that
    }
    // Strip trailing group tag like "-SYLIX" if present
    if let Some(idx) = s.rfind('-') {
        let after = &s[idx + 1..];
        if !after.is_empty() && after.chars().all(|c| c.is_ascii_alphanumeric()) && after.len() <= 12 {
            let before = s[..idx].trim();
            if !before.is_empty() {
                return before.to_string();
            }
        }
    }
    s.to_string()
}

// Matches S01E03 anywhere in string (scene releases have show name prefix)
static RE_SXXEXX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[Ss](\d+)[Ee](\d+)[\s.\-]*(.*)$").unwrap());
static RE_NNXNN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|[\s.])(\d+)x(\d+)[\s.\-]*(.*)$").unwrap());
// Compact 4-digit SSEE format: e.g. "0201" = S02E01, bounded by non-digits
static RE_COMPACT_SSEE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[.\-\s])(\d{2})(\d{2})(?:[.\-\s]|$)").unwrap());
static RE_EPISODE_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|[\s.])Episode\s+(\d+)[\s.\-]*(.*)$").unwrap());
static RE_BARE_NUM_TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)[\s.\-]+(.+)$").unwrap());
static RE_BARE_NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)$").unwrap());

pub fn parse_episode_filename(filename: &str) -> ParsedEpisodeInfo {
    // Strip extension first
    let stem = Path::new(filename).file_stem().unwrap_or_default().to_string_lossy();
    let stem = stem.trim();

    // S01E03 - Episode Title  /  S01E03.Episode.Title  /  S1E3
    if let Some(caps) = RE_SXXEXX.captures(stem) {
        let season: u32 = caps[1].parse().unwrap();
        let episode: u32 = caps[2].parse().unwrap();
        let raw_title = caps.get(3).map(|m| m.as_str()).unwrap_or("");
        let title = clean_title(raw_title);
        return ParsedEpisodeInfo {
            season: Some(season),
            episode: Some(episode),
            title: if title.is_empty() {
                format!("Episode {episode}")
            } else {
                title
            },
        };
    }

    // 01x03 - Title
    if let Some(caps) = RE_NNXNN.captures(stem) {
        let season: u32 = caps[1].parse().unwrap();
        let episode: u32 = caps[2].parse().unwrap();
        let raw_title = caps.get(3).map(|m| m.as_str()).unwrap_or("");
        let title = clean_title(raw_title);
        return ParsedEpisodeInfo {
            season: Some(season),
            episode: Some(episode),
            title: if title.is_empty() {
                format!("Episode {episode}")
            } else {
                title
            },
        };
    }

    // Compact 4-digit SSEE: "0201" = S02E01 (common in scene releases like afo-show-0201-720)
    // Only match if season 01-30 and episode 01-99 to avoid false positives on years/resolutions
    if let Some(caps) = RE_COMPACT_SSEE.captures(stem) {
        let season: u32 = caps[1].parse().unwrap();
        let episode: u32 = caps[2].parse().unwrap();
        if (1..=30).contains(&season) && episode >= 1 {
            return ParsedEpisodeInfo {
                season: Some(season),
                episode: Some(episode),
                title: format!("Episode {episode}"),
            };
        }
    }

    // Episode 03 - Title
    if let Some(caps) = RE_EPISODE_WORD.captures(stem) {
        let episode: u32 = caps[1].parse().unwrap();
        let raw_title = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        let title = clean_title(raw_title);
        return ParsedEpisodeInfo {
            season: None,
            episode: Some(episode),
            title: if title.is_empty() {
                format!("Episode {episode}")
            } else {
                title
            },
        };
    }

    // 03 - Title  (number followed by separator and title text)
    if let Some(caps) = RE_BARE_NUM_TITLE.captures(stem) {
        let episode: u32 = caps[1].parse().unwrap();
        let title = clean_title(&caps[2]);
        return ParsedEpisodeInfo {
            season: None,
            episode: Some(episode),
            title: if title.is_empty() {
                format!("Episode {episode}")
            } else {
                title
            },
        };
    }

    // 03  (bare number)
    if let Some(caps) = RE_BARE_NUM.captures(stem) {
        let episode: u32 = caps[1].parse().unwrap();
        return ParsedEpisodeInfo {
            season: None,
            episode: Some(episode),
            title: format!("Episode {episode}"),
        };
    }

    // Fallback: no parseable season/episode
    ParsedEpisodeInfo {
        season: None,
        episode: None,
        title: clean_title(stem),
    }
}

fn normalize_language(lang: &str) -> String {
    match lang.to_lowercase().as_str() {
        "en" | "eng" | "english" => "en".to_string(),
        "sv" | "swe" | "swedish" => "sv".to_string(),
        "de" | "ger" | "german" | "deu" => "de".to_string(),
        "fr" | "fre" | "french" | "fra" => "fr".to_string(),
        "es" | "spa" | "spanish" => "es".to_string(),
        "no" | "nor" | "norwegian" => "no".to_string(),
        "da" | "dan" | "danish" => "da".to_string(),
        "fi" | "fin" | "finnish" => "fi".to_string(),
        other => other.to_lowercase(),
    }
}

fn find_subtitle_files(series_path: &Path, video_stem: &str, rel_path: &str) -> Vec<SubtitleFile> {
    let mut subtitles = Vec::new();

    let entries = match std::fs::read_dir(series_path) {
        Ok(entries) => entries,
        Err(_) => return subtitles,
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("srt") {
            continue;
        }

        let srt_stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s,
            None => continue,
        };

        // Check for exact match: video_stem.srt
        if srt_stem.eq_ignore_ascii_case(video_stem) {
            let filename = entry.file_name().to_string_lossy().to_string();
            subtitles.push(SubtitleFile {
                language: "en".to_string(),
                path: format!("{rel_path}/{filename}"),
            });
            continue;
        }

        // Check for language suffix: video_stem.lang.srt
        if let Some(rest) = srt_stem.strip_prefix(video_stem) {
            if let Some(lang) = rest.strip_prefix('.') {
                if !lang.is_empty() && !lang.contains('.') {
                    let filename = entry.file_name().to_string_lossy().to_string();
                    subtitles.push(SubtitleFile {
                        language: normalize_language(lang),
                        path: format!("{rel_path}/{filename}"),
                    });
                }
            }
        }
    }

    subtitles.sort_by(|a, b| a.path.cmp(&b.path));
    subtitles
}

impl Library {
    pub fn scan(media_root: &Path) -> Result<Self, std::io::Error> {
        let mut series_map = BTreeMap::new();

        // Each direct subdirectory of media_root is a series
        let mut entries: Vec<_> = std::fs::read_dir(media_root)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();

        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let series_path = entry.path();
            let rel_path = dir_name.clone();
            let series_id = stable_id(&rel_path);

            // Find art
            let art = ART_NAMES.iter().find_map(|name| {
                let art_path = series_path.join(name);
                if art_path.exists() {
                    Some(format!("{rel_path}/{name}"))
                } else {
                    None
                }
            });

            // Find backdrop
            let backdrop = BACKDROP_NAMES.iter().find_map(|name| {
                let path = series_path.join(name);
                if path.exists() {
                    Some(format!("{rel_path}/{name}"))
                } else {
                    None
                }
            });

            // Check for tmdb.txt override
            let tmdb_id_override = {
                let tmdb_path = series_path.join("tmdb.txt");
                if tmdb_path.exists() {
                    std::fs::read_to_string(&tmdb_path)
                        .ok()
                        .and_then(|s| s.trim().parse::<u64>().ok())
                } else {
                    None
                }
            };

            // Collect episodes (video files in the series directory)
            let mut video_files: Vec<_> = std::fs::read_dir(&series_path)?
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false) && is_video(&e.path()))
                .collect();

            video_files.sort_by_key(|e| e.file_name());

            // Skip folders that aren't series (no videos, no art, no tmdb.txt)
            if video_files.is_empty() && art.is_none() && backdrop.is_none() && tmdb_id_override.is_none() {
                continue;
            }

            let episodes: Vec<Episode> = video_files
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let filename = f.file_name().to_string_lossy().to_string();
                    let ep_rel_path = format!("{rel_path}/{filename}");
                    let size = f.metadata().map(|m| m.len()).unwrap_or(0);

                    let parsed = parse_episode_filename(&filename);

                    let video_stem = Path::new(&filename)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let subtitles = find_subtitle_files(&series_path, &video_stem, &rel_path);

                    Episode {
                        id: stable_id(&ep_rel_path),
                        title: parsed.title,
                        path: ep_rel_path,
                        filename,
                        size_bytes: size,
                        index: i,
                        season_number: parsed.season,
                        episode_number: parsed.episode,
                        subtitles,
                    }
                })
                .collect();

            series_map.insert(
                series_id.clone(),
                Series {
                    id: series_id,
                    title: dir_name,
                    path: rel_path,
                    art,
                    backdrop,
                    tmdb_id_override,
                    episodes,
                },
            );
        }

        Ok(Library { series: series_map })
    }

    pub fn find_episode(&self, episode_id: &str) -> Option<(&Series, &Episode)> {
        for series in self.series.values() {
            if let Some(ep) = series.episodes.iter().find(|e| e.id == episode_id) {
                return Some((series, ep));
            }
        }
        None
    }

    pub fn find_series(&self, series_id: &str) -> Option<&Series> {
        self.series.get(series_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_media_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn scan_empty_directory() {
        let dir = make_media_dir();
        let lib = Library::scan(dir.path()).unwrap();
        assert!(lib.series.is_empty());
    }

    #[test]
    fn scan_with_series_and_episodes() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Breaking Bad");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("S01E01.mp4"), b"video1").unwrap();
        fs::write(series_dir.join("S01E02.mkv"), b"video2").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        assert_eq!(lib.series.len(), 1);

        let series = lib.series.values().next().unwrap();
        assert_eq!(series.title, "Breaking Bad");
        assert_eq!(series.episodes.len(), 2);
        assert_eq!(series.episodes[0].title, "Episode 1");
        assert_eq!(series.episodes[0].season_number, Some(1));
        assert_eq!(series.episodes[0].episode_number, Some(1));
        assert_eq!(series.episodes[1].title, "Episode 2");
        assert_eq!(series.episodes[1].season_number, Some(1));
        assert_eq!(series.episodes[1].episode_number, Some(2));
        assert_eq!(series.episodes[0].index, 0);
        assert_eq!(series.episodes[1].index, 1);
    }

    #[test]
    fn non_video_files_are_ignored() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("episode.mp4"), b"video").unwrap();
        fs::write(series_dir.join("notes.txt"), b"text").unwrap();
        fs::write(series_dir.join("subtitles.srt"), b"subs").unwrap();
        fs::write(series_dir.join("info.nfo"), b"info").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.episodes.len(), 1);
        assert_eq!(series.episodes[0].filename, "episode.mp4");
    }

    #[test]
    fn series_with_only_non_video_files_is_excluded() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("EmptySeries");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("readme.txt"), b"nothing").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        assert!(lib.series.is_empty());
    }

    #[test]
    fn art_detection_poster_jpg() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("episode.mp4"), b"video").unwrap();
        fs::write(series_dir.join("poster.jpg"), b"image").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.art.as_deref(), Some("Show/poster.jpg"));
    }

    #[test]
    fn art_detection_cover_png() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("episode.mp4"), b"video").unwrap();
        fs::write(series_dir.join("cover.png"), b"image").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.art.as_deref(), Some("Show/cover.png"));
    }

    #[test]
    fn art_detection_priority_order() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("episode.mp4"), b"video").unwrap();
        // poster.jpg should win over cover.png due to ART_NAMES order
        fs::write(series_dir.join("poster.jpg"), b"image1").unwrap();
        fs::write(series_dir.join("cover.png"), b"image2").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.art.as_deref(), Some("Show/poster.jpg"));
    }

    #[test]
    fn no_art_when_absent() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("episode.mp4"), b"video").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert!(series.art.is_none());
    }

    #[test]
    fn stable_id_same_path_same_id() {
        let id1 = stable_id("Show/S01E01.mp4");
        let id2 = stable_id("Show/S01E01.mp4");
        assert_eq!(id1, id2);
    }

    #[test]
    fn stable_id_different_paths_different_ids() {
        let id1 = stable_id("Show/S01E01.mp4");
        let id2 = stable_id("Show/S01E02.mp4");
        assert_ne!(id1, id2);
    }

    #[test]
    fn stable_id_is_12_chars() {
        let id = stable_id("anything");
        assert_eq!(id.len(), 12);
    }

    #[test]
    fn find_episode_works() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("ep1.mp4"), b"video").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        let ep_id = &series.episodes[0].id;

        let (found_series, found_ep) = lib.find_episode(ep_id).unwrap();
        assert_eq!(found_series.title, "Show");
        assert_eq!(found_ep.title, "ep1");
    }

    #[test]
    fn find_episode_returns_none_for_unknown() {
        let dir = make_media_dir();
        let lib = Library::scan(dir.path()).unwrap();
        assert!(lib.find_episode("nonexistent").is_none());
    }

    #[test]
    fn find_series_works() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("MyShow");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("ep.mp4"), b"video").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        let found = lib.find_series(&series.id).unwrap();
        assert_eq!(found.title, "MyShow");
    }

    #[test]
    fn find_series_returns_none_for_unknown() {
        let dir = make_media_dir();
        let lib = Library::scan(dir.path()).unwrap();
        assert!(lib.find_series("nonexistent").is_none());
    }

    #[test]
    fn all_video_extensions_detected() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        for ext in &["mp4", "m4v", "mov", "mkv", "avi", "webm"] {
            fs::write(series_dir.join(format!("file.{}", ext)), b"data").unwrap();
        }

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.episodes.len(), 6);
    }

    #[test]
    fn episodes_sorted_by_filename() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("c_third.mp4"), b"v").unwrap();
        fs::write(series_dir.join("a_first.mp4"), b"v").unwrap();
        fs::write(series_dir.join("b_second.mp4"), b"v").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.episodes[0].filename, "a_first.mp4");
        assert_eq!(series.episodes[1].filename, "b_second.mp4");
        assert_eq!(series.episodes[2].filename, "c_third.mp4");
    }

    #[test]
    fn multiple_series_scanned() {
        let dir = make_media_dir();
        for name in &["Alpha", "Beta", "Gamma"] {
            let series_dir = dir.path().join(name);
            fs::create_dir(&series_dir).unwrap();
            fs::write(series_dir.join("ep.mp4"), b"v").unwrap();
        }

        let lib = Library::scan(dir.path()).unwrap();
        assert_eq!(lib.series.len(), 3);
    }

    // --- parse_episode_filename tests ---

    #[test]
    fn parse_sxxexx_with_dash_title() {
        let info = parse_episode_filename("S01E03 - Episode Title.mp4");
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(3));
        assert_eq!(info.title, "Episode Title");
    }

    #[test]
    fn parse_sxxexx_with_dot_title() {
        let info = parse_episode_filename("S01E03.Episode.Title.mp4");
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(3));
        assert_eq!(info.title, "Episode Title");
    }

    #[test]
    fn parse_sxxexx_with_space_title() {
        let info = parse_episode_filename("S01E03 Episode Title.mp4");
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(3));
        assert_eq!(info.title, "Episode Title");
    }

    #[test]
    fn parse_sxxexx_no_title() {
        let info = parse_episode_filename("S1E3.mp4");
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(3));
        assert_eq!(info.title, "Episode 3");
    }

    #[test]
    fn parse_nnxnn_with_title() {
        let info = parse_episode_filename("01x03 - Title.mp4");
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(3));
        assert_eq!(info.title, "Title");
    }

    #[test]
    fn parse_episode_word_with_title() {
        let info = parse_episode_filename("Episode 03 - Title.mp4");
        assert_eq!(info.season, None);
        assert_eq!(info.episode, Some(3));
        assert_eq!(info.title, "Title");
    }

    #[test]
    fn parse_bare_number_with_title() {
        let info = parse_episode_filename("03 - Title.mp4");
        assert_eq!(info.season, None);
        assert_eq!(info.episode, Some(3));
        assert_eq!(info.title, "Title");
    }

    #[test]
    fn parse_bare_number_only() {
        let info = parse_episode_filename("03.mp4");
        assert_eq!(info.season, None);
        assert_eq!(info.episode, Some(3));
        assert_eq!(info.title, "Episode 3");
    }

    #[test]
    fn parse_fallback_plain_title() {
        let info = parse_episode_filename("My Great Video.mp4");
        assert_eq!(info.season, None);
        assert_eq!(info.episode, None);
        assert_eq!(info.title, "My Great Video");
    }

    #[test]
    fn parse_sxxexx_case_insensitive() {
        let info = parse_episode_filename("s02e10 - Finale.mkv");
        assert_eq!(info.season, Some(2));
        assert_eq!(info.episode, Some(10));
        assert_eq!(info.title, "Finale");
    }

    #[test]
    fn parse_large_numbers() {
        let info = parse_episode_filename("S12E199 - Big Number.mp4");
        assert_eq!(info.season, Some(12));
        assert_eq!(info.episode, Some(199));
        assert_eq!(info.title, "Big Number");
    }

    #[test]
    fn parse_dotted_fallback() {
        let info = parse_episode_filename("some.dotted.name.mp4");
        assert_eq!(info.season, None);
        assert_eq!(info.episode, None);
        assert_eq!(info.title, "some dotted name");
    }

    #[test]
    fn parse_episode_word_no_title() {
        let info = parse_episode_filename("Episode 05.mp4");
        assert_eq!(info.season, None);
        assert_eq!(info.episode, Some(5));
        assert_eq!(info.title, "Episode 5");
    }

    #[test]
    fn parse_nnxnn_no_title() {
        let info = parse_episode_filename("02x07.mkv");
        assert_eq!(info.season, Some(2));
        assert_eq!(info.episode, Some(7));
        assert_eq!(info.title, "Episode 7");
    }

    #[test]
    fn parse_scene_release_sxxexx() {
        let info = parse_episode_filename("will trent s04e13 720p web h264-sylix.mkv");
        assert_eq!(info.season, Some(4));
        assert_eq!(info.episode, Some(13));
        // Title stripped of release tags
        assert_eq!(info.title, "Episode 13");
    }

    #[test]
    fn parse_scene_release_prefix_sxxexx() {
        let info = parse_episode_filename("Show.Name.S02E05.Episode.Title.720p.WEB.x264.mkv");
        assert_eq!(info.season, Some(2));
        assert_eq!(info.episode, Some(5));
        assert_eq!(info.title, "Episode Title");
    }

    #[test]
    fn parse_scene_release_dashed() {
        let info = parse_episode_filename("afo-ddbornagain-s02e01-720-web.mkv");
        assert_eq!(info.season, Some(2));
        assert_eq!(info.episode, Some(1));
    }

    #[test]
    fn parse_compact_ssee_format() {
        let info = parse_episode_filename("afo-ddbornagain-0201-720-web.mkv");
        assert_eq!(info.season, Some(2));
        assert_eq!(info.episode, Some(1));
        assert_eq!(info.title, "Episode 1");
    }

    #[test]
    fn parse_compact_ssee_higher_numbers() {
        let info = parse_episode_filename("show-0412-720p.mkv");
        assert_eq!(info.season, Some(4));
        assert_eq!(info.episode, Some(12));
    }

    #[test]
    fn tmdb_txt_override_detected() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Some Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("ep.mp4"), b"video").unwrap();
        fs::write(series_dir.join("tmdb.txt"), "12345\n").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.tmdb_id_override, Some(12345));
    }

    #[test]
    fn tmdb_txt_absent_gives_none() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("ep.mp4"), b"video").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.tmdb_id_override, None);
    }

    #[test]
    fn tmdb_txt_invalid_content_gives_none() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("ep.mp4"), b"video").unwrap();
        fs::write(series_dir.join("tmdb.txt"), "not-a-number").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.tmdb_id_override, None);
    }

    // --- subtitle detection tests ---

    #[test]
    fn subtitle_matching_srt_detected() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("S01E01.mp4"), b"video").unwrap();
        fs::write(series_dir.join("S01E01.srt"), b"subs").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.episodes[0].subtitles.len(), 1);
        assert_eq!(series.episodes[0].subtitles[0].language, "en");
        assert_eq!(series.episodes[0].subtitles[0].path, "Show/S01E01.srt");
    }

    #[test]
    fn subtitle_language_suffix_detected() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("S01E01.mp4"), b"video").unwrap();
        fs::write(series_dir.join("S01E01.en.srt"), b"subs").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.episodes[0].subtitles.len(), 1);
        assert_eq!(series.episodes[0].subtitles[0].language, "en");
        assert_eq!(series.episodes[0].subtitles[0].path, "Show/S01E01.en.srt");
    }

    #[test]
    fn subtitle_multiple_files_detected() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("S01E01.mp4"), b"video").unwrap();
        fs::write(series_dir.join("S01E01.srt"), b"subs").unwrap();
        fs::write(series_dir.join("S01E01.sv.srt"), b"subs sv").unwrap();
        fs::write(series_dir.join("S01E01.french.srt"), b"subs fr").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert_eq!(series.episodes[0].subtitles.len(), 3);

        let langs: Vec<&str> = series.episodes[0]
            .subtitles
            .iter()
            .map(|s| s.language.as_str())
            .collect();
        assert!(langs.contains(&"en"));
        assert!(langs.contains(&"sv"));
        assert!(langs.contains(&"fr"));
    }

    #[test]
    fn subtitle_no_srt_gives_empty_vec() {
        let dir = make_media_dir();
        let series_dir = dir.path().join("Show");
        fs::create_dir(&series_dir).unwrap();
        fs::write(series_dir.join("S01E01.mp4"), b"video").unwrap();

        let lib = Library::scan(dir.path()).unwrap();
        let series = lib.series.values().next().unwrap();
        assert!(series.episodes[0].subtitles.is_empty());
    }
}
