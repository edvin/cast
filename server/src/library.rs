use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use uuid::Uuid;

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "mkv", "avi", "webm"];
const ART_NAMES: &[&str] = &[
    "poster.jpg",
    "poster.png",
    "folder.jpg",
    "folder.png",
    "cover.jpg",
    "cover.png",
    "banner.jpg",
    "banner.png",
];
const BACKDROP_NAMES: &[&str] = &["backdrop.jpg", "backdrop.png", "fanart.jpg", "fanart.png"];

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

            // Collect episodes (video files in the series directory)
            let mut video_files: Vec<_> = std::fs::read_dir(&series_path)?
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false) && is_video(&e.path()))
                .collect();

            video_files.sort_by_key(|e| e.file_name());

            if video_files.is_empty() {
                continue;
            }

            let episodes: Vec<Episode> = video_files
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let filename = f.file_name().to_string_lossy().to_string();
                    let ep_rel_path = format!("{rel_path}/{filename}");
                    let size = f.metadata().map(|m| m.len()).unwrap_or(0);

                    // Derive a display title from filename (strip extension)
                    let title = Path::new(&filename)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    Episode {
                        id: stable_id(&ep_rel_path),
                        title,
                        path: ep_rel_path,
                        filename,
                        size_bytes: size,
                        index: i,
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
        assert_eq!(series.episodes[0].title, "S01E01");
        assert_eq!(series.episodes[1].title, "S01E02");
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
}
