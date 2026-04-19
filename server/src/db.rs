use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

pub struct Database {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WatchProgress {
    pub episode_id: String,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub completed: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeriesMetadata {
    pub series_id: String,
    pub tmdb_id: Option<u64>,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub first_air_date: Option<String>,
    pub genres: Option<String>,
    pub rating: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MovieMetadata {
    pub movie_id: String,
    pub tmdb_id: Option<u64>,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub release_date: Option<String>,
    pub runtime_minutes: Option<u32>,
    pub genres: Option<String>,
    pub rating: Option<f64>,
    pub tagline: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpisodeMetadata {
    pub episode_id: String,
    pub series_id: String,
    pub tmdb_episode_id: Option<u64>,
    pub season_number: Option<u32>,
    pub episode_number: Option<u32>,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub runtime_minutes: Option<u32>,
    pub still_url: Option<String>,
}

impl Database {
    pub fn new(media_path: &Path) -> Result<Self, rusqlite::Error> {
        let db_path = media_path.join("cast.db");
        let conn = Connection::open(db_path)?;

        // Best-effort idempotent migrations for columns added to existing tables.
        // ALTER TABLE ADD COLUMN only succeeds once; ignore "duplicate column" errors
        // so reopening a v0.4.17 database still works.
        for stmt in [
            "ALTER TABLE tmdb_lookup_attempts ADD COLUMN title TEXT",
            "ALTER TABLE tmdb_lookup_attempts ADD COLUMN path TEXT",
        ] {
            let _ = conn.execute(stmt, []);
        }

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS watch_progress (
                episode_id TEXT PRIMARY KEY,
                position_secs REAL NOT NULL DEFAULT 0,
                duration_secs REAL NOT NULL DEFAULT 0,
                completed INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS series_metadata (
                series_id TEXT PRIMARY KEY,
                tmdb_id INTEGER,
                title TEXT,
                overview TEXT,
                first_air_date TEXT,
                genres TEXT,
                rating REAL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS episode_metadata (
                episode_id TEXT NOT NULL,
                series_id TEXT NOT NULL,
                tmdb_episode_id INTEGER,
                season_number INTEGER,
                episode_number INTEGER,
                title TEXT,
                overview TEXT,
                air_date TEXT,
                runtime_minutes INTEGER,
                still_url TEXT,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (series_id, season_number, episode_number)
            );

            CREATE TABLE IF NOT EXISTS episode_credits (
                series_id TEXT NOT NULL,
                season_number INTEGER NOT NULL,
                episode_number INTEGER NOT NULL,
                credits_json TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (series_id, season_number, episode_number)
            );

            CREATE TABLE IF NOT EXISTS remux_failures (
                episode_id TEXT PRIMARY KEY,
                series_id TEXT NOT NULL,
                path TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                last_attempt_at TEXT NOT NULL DEFAULT (datetime('now')),
                given_up INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS movie_metadata (
                movie_id TEXT PRIMARY KEY,
                tmdb_id INTEGER,
                title TEXT,
                overview TEXT,
                release_date TEXT,
                runtime_minutes INTEGER,
                genres TEXT,
                rating REAL,
                tagline TEXT,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS tmdb_lookup_attempts (
                content_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                title TEXT,
                path TEXT,
                attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                last_attempt_at TEXT NOT NULL DEFAULT (datetime('now')),
                given_up INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS artwork (
                content_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                content_type TEXT NOT NULL,
                bytes BLOB NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (content_id, kind)
            );",
        )?;

        Ok(Database { conn: Mutex::new(conn) })
    }

    pub fn get_progress(&self, episode_id: &str) -> Option<WatchProgress> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT episode_id, position_secs, duration_secs, completed, updated_at
             FROM watch_progress WHERE episode_id = ?1",
            params![episode_id],
            |row| {
                Ok(WatchProgress {
                    episode_id: row.get(0)?,
                    position_secs: row.get(1)?,
                    duration_secs: row.get(2)?,
                    completed: row.get::<_, i32>(3)? != 0,
                    updated_at: row.get(4)?,
                })
            },
        )
        .ok()
    }

    pub fn get_all_progress(&self) -> Vec<WatchProgress> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT episode_id, position_secs, duration_secs, completed, updated_at
                 FROM watch_progress ORDER BY updated_at DESC",
            )
            .unwrap();

        stmt.query_map([], |row| {
            Ok(WatchProgress {
                episode_id: row.get(0)?,
                position_secs: row.get(1)?,
                duration_secs: row.get(2)?,
                completed: row.get::<_, i32>(3)? != 0,
                updated_at: row.get(4)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn get_series_progress(&self, episode_ids: &[String]) -> Vec<WatchProgress> {
        let conn = self.conn.lock().unwrap();
        let placeholders: Vec<String> = episode_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let sql = format!(
            "SELECT episode_id, position_secs, duration_secs, completed, updated_at
             FROM watch_progress WHERE episode_id IN ({}) ORDER BY updated_at DESC",
            placeholders.join(", ")
        );

        let params: Vec<&dyn rusqlite::types::ToSql> =
            episode_ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();

        let mut stmt = conn.prepare(&sql).unwrap();
        stmt.query_map(params.as_slice(), |row| {
            Ok(WatchProgress {
                episode_id: row.get(0)?,
                position_secs: row.get(1)?,
                duration_secs: row.get(2)?,
                completed: row.get::<_, i32>(3)? != 0,
                updated_at: row.get(4)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn update_progress(
        &self,
        episode_id: &str,
        position_secs: f64,
        duration_secs: f64,
    ) -> Result<(), rusqlite::Error> {
        let completed = duration_secs > 0.0 && (position_secs / duration_secs) >= 0.9;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO watch_progress (episode_id, position_secs, duration_secs, completed, updated_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(episode_id) DO UPDATE SET
                position_secs = ?2,
                duration_secs = ?3,
                completed = ?4,
                updated_at = datetime('now')",
            params![episode_id, position_secs, duration_secs, completed as i32],
        )?;
        Ok(())
    }

    pub fn save_series_metadata(&self, meta: &SeriesMetadata) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO series_metadata
                (series_id, tmdb_id, title, overview, first_air_date, genres, rating, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
            params![
                meta.series_id,
                meta.tmdb_id.map(|v| v as i64),
                meta.title,
                meta.overview,
                meta.first_air_date,
                meta.genres,
                meta.rating,
            ],
        )?;
        Ok(())
    }

    /// Load all series metadata in a single query, keyed by series_id
    pub fn get_all_series_metadata(&self) -> std::collections::HashMap<String, SeriesMetadata> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT series_id, tmdb_id, title, overview, first_air_date, genres, rating
                 FROM series_metadata",
            )
            .unwrap();

        stmt.query_map([], |row| {
            Ok(SeriesMetadata {
                series_id: row.get(0)?,
                tmdb_id: row.get::<_, Option<i64>>(1)?.map(|v| v as u64),
                title: row.get(2)?,
                overview: row.get(3)?,
                first_air_date: row.get(4)?,
                genres: row.get(5)?,
                rating: row.get(6)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .map(|m| (m.series_id.clone(), m))
        .collect()
    }

    /// Load all watch progress in a single query, keyed by episode_id
    pub fn get_all_progress_map(&self) -> std::collections::HashMap<String, WatchProgress> {
        self.get_all_progress()
            .into_iter()
            .map(|p| (p.episode_id.clone(), p))
            .collect()
    }

    /// Load all episode metadata in a single query, keyed by (series_id, season, episode)
    pub fn get_all_episode_metadata(&self) -> std::collections::HashMap<(String, u32, u32), EpisodeMetadata> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT episode_id, series_id, tmdb_episode_id, season_number, episode_number,
                        title, overview, air_date, runtime_minutes, still_url
                 FROM episode_metadata",
            )
            .unwrap();

        stmt.query_map([], |row| {
            Ok(EpisodeMetadata {
                episode_id: row.get(0)?,
                series_id: row.get(1)?,
                tmdb_episode_id: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                season_number: row.get::<_, Option<i64>>(3)?.map(|v| v as u32),
                episode_number: row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                title: row.get(5)?,
                overview: row.get(6)?,
                air_date: row.get(7)?,
                runtime_minutes: row.get::<_, Option<i64>>(8)?.map(|v| v as u32),
                still_url: row.get(9)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .filter_map(|m| {
            let key = (m.series_id.clone(), m.season_number?, m.episode_number?);
            Some((key, m))
        })
        .collect()
    }

    pub fn get_series_metadata(&self, series_id: &str) -> Option<SeriesMetadata> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT series_id, tmdb_id, title, overview, first_air_date, genres, rating
             FROM series_metadata WHERE series_id = ?1",
            params![series_id],
            |row| {
                Ok(SeriesMetadata {
                    series_id: row.get(0)?,
                    tmdb_id: row.get::<_, Option<i64>>(1)?.map(|v| v as u64),
                    title: row.get(2)?,
                    overview: row.get(3)?,
                    first_air_date: row.get(4)?,
                    genres: row.get(5)?,
                    rating: row.get(6)?,
                })
            },
        )
        .ok()
    }

    pub fn save_episode_metadata(&self, meta: &EpisodeMetadata) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO episode_metadata
                (episode_id, series_id, tmdb_episode_id, season_number, episode_number,
                 title, overview, air_date, runtime_minutes, still_url, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
             ON CONFLICT(series_id, season_number, episode_number) DO UPDATE SET
                tmdb_episode_id = ?3, title = ?6, overview = ?7, air_date = ?8,
                runtime_minutes = ?9, still_url = ?10, updated_at = datetime('now')",
            params![
                meta.episode_id,
                meta.series_id,
                meta.tmdb_episode_id.map(|v| v as i64),
                meta.season_number.map(|v| v as i64),
                meta.episode_number.map(|v| v as i64),
                meta.title,
                meta.overview,
                meta.air_date,
                meta.runtime_minutes.map(|v| v as i64),
                meta.still_url,
            ],
        )?;
        Ok(())
    }

    pub fn get_episode_metadata(&self, episode_id: &str) -> Option<EpisodeMetadata> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT episode_id, series_id, tmdb_episode_id, season_number, episode_number,
                    title, overview, air_date, runtime_minutes, still_url
             FROM episode_metadata WHERE episode_id = ?1",
            params![episode_id],
            |row| {
                Ok(EpisodeMetadata {
                    episode_id: row.get(0)?,
                    series_id: row.get(1)?,
                    tmdb_episode_id: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                    season_number: row.get::<_, Option<i64>>(3)?.map(|v| v as u32),
                    episode_number: row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                    title: row.get(5)?,
                    overview: row.get(6)?,
                    air_date: row.get(7)?,
                    runtime_minutes: row.get::<_, Option<i64>>(8)?.map(|v| v as u32),
                    still_url: row.get(9)?,
                })
            },
        )
        .ok()
    }

    pub fn get_episode_metadata_by_number(
        &self,
        series_id: &str,
        season: u32,
        episode: u32,
    ) -> Option<EpisodeMetadata> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT episode_id, series_id, tmdb_episode_id, season_number, episode_number,
                    title, overview, air_date, runtime_minutes, still_url
             FROM episode_metadata WHERE series_id = ?1 AND season_number = ?2 AND episode_number = ?3",
            params![series_id, season as i64, episode as i64],
            |row| {
                Ok(EpisodeMetadata {
                    episode_id: row.get(0)?,
                    series_id: row.get(1)?,
                    tmdb_episode_id: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                    season_number: row.get::<_, Option<i64>>(3)?.map(|v| v as u32),
                    episode_number: row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                    title: row.get(5)?,
                    overview: row.get(6)?,
                    air_date: row.get(7)?,
                    runtime_minutes: row.get::<_, Option<i64>>(8)?.map(|v| v as u32),
                    still_url: row.get(9)?,
                })
            },
        )
        .ok()
    }

    pub fn delete_series_metadata(&self, series_id: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("DELETE FROM series_metadata WHERE series_id = ?1", params![series_id]);
        let _ = conn.execute("DELETE FROM episode_metadata WHERE series_id = ?1", params![series_id]);
        let _ = conn.execute("DELETE FROM episode_credits WHERE series_id = ?1", params![series_id]);
        let _ = conn.execute("DELETE FROM remux_failures WHERE series_id = ?1", params![series_id]);
    }

    // --- Movie metadata ---

    pub fn save_movie_metadata(&self, meta: &MovieMetadata) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO movie_metadata (movie_id, tmdb_id, title, overview, release_date,
                                         runtime_minutes, genres, rating, tagline, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
             ON CONFLICT(movie_id) DO UPDATE SET
                tmdb_id = excluded.tmdb_id,
                title = excluded.title,
                overview = excluded.overview,
                release_date = excluded.release_date,
                runtime_minutes = excluded.runtime_minutes,
                genres = excluded.genres,
                rating = excluded.rating,
                tagline = excluded.tagline,
                updated_at = excluded.updated_at",
            params![
                meta.movie_id,
                meta.tmdb_id,
                meta.title,
                meta.overview,
                meta.release_date,
                meta.runtime_minutes,
                meta.genres,
                meta.rating,
                meta.tagline,
            ],
        )?;
        Ok(())
    }

    pub fn get_movie_metadata(&self, movie_id: &str) -> Option<MovieMetadata> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT movie_id, tmdb_id, title, overview, release_date,
                    runtime_minutes, genres, rating, tagline
             FROM movie_metadata WHERE movie_id = ?1",
            params![movie_id],
            |row| {
                Ok(MovieMetadata {
                    movie_id: row.get(0)?,
                    tmdb_id: row.get::<_, Option<i64>>(1)?.map(|v| v as u64),
                    title: row.get(2)?,
                    overview: row.get(3)?,
                    release_date: row.get(4)?,
                    runtime_minutes: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
                    genres: row.get(6)?,
                    rating: row.get(7)?,
                    tagline: row.get(8)?,
                })
            },
        )
        .ok()
    }

    pub fn delete_movie_metadata(&self, movie_id: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("DELETE FROM movie_metadata WHERE movie_id = ?1", params![movie_id]);
    }

    // --- TMDB lookup attempt tracking ---

    /// Record a failed TMDB lookup attempt and flip `given_up` once the attempt count
    /// crosses `max_attempts`. Kind is "series" or "movie".
    pub fn record_tmdb_failure(
        &self,
        content_id: &str,
        kind: &str,
        title: &str,
        path: &str,
        error: &str,
        max_attempts: i64,
    ) -> TmdbLookupState {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO tmdb_lookup_attempts (content_id, kind, title, path, attempts, last_error, last_attempt_at, given_up)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, datetime('now'), 0)
             ON CONFLICT(content_id) DO UPDATE SET
                 kind = excluded.kind,
                 title = excluded.title,
                 path = excluded.path,
                 attempts = attempts + 1,
                 last_error = excluded.last_error,
                 last_attempt_at = excluded.last_attempt_at",
            params![content_id, kind, title, path, error],
        );
        let attempts: i64 = conn
            .query_row(
                "SELECT attempts FROM tmdb_lookup_attempts WHERE content_id = ?1",
                params![content_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let given_up = attempts >= max_attempts;
        if given_up {
            let _ = conn.execute(
                "UPDATE tmdb_lookup_attempts SET given_up = 1 WHERE content_id = ?1",
                params![content_id],
            );
        }
        TmdbLookupState {
            attempts: attempts as u32,
            given_up,
        }
    }

    /// On a successful match, drop the attempt row so a future regression retries cleanly.
    pub fn clear_tmdb_failure(&self, content_id: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "DELETE FROM tmdb_lookup_attempts WHERE content_id = ?1",
            params![content_id],
        );
    }

    pub fn is_tmdb_lookup_abandoned(&self, content_id: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT given_up FROM tmdb_lookup_attempts WHERE content_id = ?1",
            params![content_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|v| v != 0)
        .unwrap_or(false)
    }

    /// Clear all abandoned TMDB lookup flags (user-triggered "retry all metadata").
    pub fn retry_tmdb_lookups(&self) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("DELETE FROM tmdb_lookup_attempts", []);
    }

    pub fn list_tmdb_failures(&self) -> Vec<TmdbLookupFailure> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT content_id, kind, title, path, attempts, last_error, last_attempt_at, given_up
             FROM tmdb_lookup_attempts ORDER BY given_up DESC, last_attempt_at DESC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let iter = stmt.query_map([], |row| {
            Ok(TmdbLookupFailure {
                content_id: row.get(0)?,
                kind: row.get(1)?,
                title: row.get(2)?,
                path: row.get(3)?,
                attempts: row.get::<_, i64>(4)? as u32,
                last_error: row.get(5)?,
                last_attempt_at: row.get(6)?,
                given_up: row.get::<_, i64>(7)? != 0,
            })
        });
        match iter {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Drop any failure rows whose content_id is no longer present in the live library.
    /// Called from the periodic rescan so entries don't linger after the user removes a
    /// file or folder from disk.
    pub fn prune_orphan_failures(&self, known_ids: &std::collections::HashSet<String>) {
        let conn = self.conn.lock().unwrap();

        fn collect_ids(conn: &Connection, sql: &str) -> Vec<String> {
            let mut stmt = match conn.prepare(sql) {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
            stmt.query_map([], |row| row.get::<_, String>(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        }

        let tmdb_ids = collect_ids(&conn, "SELECT content_id FROM tmdb_lookup_attempts");
        for id in tmdb_ids {
            if !known_ids.contains(&id) {
                let _ = conn.execute("DELETE FROM tmdb_lookup_attempts WHERE content_id = ?1", params![id]);
            }
        }
        let remux_ids = collect_ids(&conn, "SELECT episode_id FROM remux_failures");
        for id in remux_ids {
            if !known_ids.contains(&id) {
                let _ = conn.execute("DELETE FROM remux_failures WHERE episode_id = ?1", params![id]);
            }
        }
        let artwork_ids = collect_ids(&conn, "SELECT DISTINCT content_id FROM artwork");
        for id in artwork_ids {
            if !known_ids.contains(&id) {
                let _ = conn.execute("DELETE FROM artwork WHERE content_id = ?1", params![id]);
            }
        }
    }

    // --- Artwork (poster/backdrop) BLOB storage ---

    /// Store poster or backdrop bytes for a content item. `kind` is "art" or "backdrop".
    pub fn save_artwork(
        &self,
        content_id: &str,
        kind: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO artwork (content_id, kind, content_type, bytes, updated_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(content_id, kind) DO UPDATE SET
                 content_type = excluded.content_type,
                 bytes = excluded.bytes,
                 updated_at = excluded.updated_at",
            params![content_id, kind, content_type, bytes],
        )?;
        Ok(())
    }

    pub fn get_artwork(&self, content_id: &str, kind: &str) -> Option<(String, Vec<u8>)> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT content_type, bytes FROM artwork WHERE content_id = ?1 AND kind = ?2",
            params![content_id, kind],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .ok()
    }

    pub fn has_artwork(&self, content_id: &str, kind: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT 1 FROM artwork WHERE content_id = ?1 AND kind = ?2",
            params![content_id, kind],
            |_row| Ok(()),
        )
        .is_ok()
    }

    pub fn delete_artwork(&self, content_id: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("DELETE FROM artwork WHERE content_id = ?1", params![content_id]);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TmdbLookupState {
    pub attempts: u32,
    pub given_up: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TmdbLookupFailure {
    pub content_id: String,
    pub kind: String,
    pub title: Option<String>,
    pub path: Option<String>,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub last_attempt_at: String,
    pub given_up: bool,
}

impl Database {
    // --- Remux failure tracking ---

    /// Record a failed remux attempt, incrementing the attempt counter and flipping
    /// `given_up` once it crosses the threshold. Returns the new state so the caller
    /// can log "Giving up..." exactly once.
    pub fn record_remux_failure(
        &self,
        episode_id: &str,
        series_id: &str,
        path: &str,
        error: &str,
        max_attempts: i64,
    ) -> RemuxFailureState {
        let conn = self.conn.lock().unwrap();
        // UPSERT: insert with attempts=1 or increment existing row.
        let _ = conn.execute(
            "INSERT INTO remux_failures (episode_id, series_id, path, attempts, last_error, last_attempt_at, given_up)
             VALUES (?1, ?2, ?3, 1, ?4, datetime('now'), 0)
             ON CONFLICT(episode_id) DO UPDATE SET
                 series_id = excluded.series_id,
                 path = excluded.path,
                 attempts = attempts + 1,
                 last_error = excluded.last_error,
                 last_attempt_at = excluded.last_attempt_at",
            params![episode_id, series_id, path, error],
        );
        let attempts: i64 = conn
            .query_row(
                "SELECT attempts FROM remux_failures WHERE episode_id = ?1",
                params![episode_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let crossing_threshold = attempts >= max_attempts;
        if crossing_threshold {
            let _ = conn.execute(
                "UPDATE remux_failures SET given_up = 1 WHERE episode_id = ?1",
                params![episode_id],
            );
        }
        RemuxFailureState {
            attempts: attempts as u32,
            given_up: crossing_threshold,
        }
    }

    /// Clear the failure record for an episode — call this on successful remux so
    /// the file doesn't stay flagged.
    pub fn clear_remux_failure(&self, episode_id: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("DELETE FROM remux_failures WHERE episode_id = ?1", params![episode_id]);
    }

    /// Is this episode marked as abandoned?
    pub fn is_remux_abandoned(&self, episode_id: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT given_up FROM remux_failures WHERE episode_id = ?1",
            params![episode_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|v| v != 0)
        .unwrap_or(false)
    }

    /// List all remux failures (abandoned first, then recent).
    pub fn list_remux_failures(&self) -> Vec<RemuxFailure> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT episode_id, series_id, path, attempts, last_error, last_attempt_at, given_up
             FROM remux_failures
             ORDER BY given_up DESC, last_attempt_at DESC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let iter = stmt.query_map([], |row| {
            Ok(RemuxFailure {
                episode_id: row.get(0)?,
                series_id: row.get(1)?,
                path: row.get(2)?,
                attempts: row.get::<_, i64>(3)? as u32,
                last_error: row.get(4)?,
                last_attempt_at: row.get(5)?,
                given_up: row.get::<_, i64>(6)? != 0,
            })
        });
        match iter {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Clear the given_up flag on all (or a specific episode) so the scanner retries.
    pub fn retry_remux_failures(&self, episode_id: Option<&str>) {
        let conn = self.conn.lock().unwrap();
        match episode_id {
            Some(id) => {
                let _ = conn.execute("DELETE FROM remux_failures WHERE episode_id = ?1", params![id]);
            }
            None => {
                let _ = conn.execute("DELETE FROM remux_failures", []);
            }
        }
    }

    /// Clear failures for a whole series — used when the user manually triggers
    /// "Remux All" on a series, so any abandoned files get re-queued.
    pub fn retry_remux_failures_for_series(&self, series_id: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("DELETE FROM remux_failures WHERE series_id = ?1", params![series_id]);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RemuxFailureState {
    pub attempts: u32,
    pub given_up: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemuxFailure {
    pub episode_id: String,
    pub series_id: String,
    pub path: String,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub last_attempt_at: String,
    pub given_up: bool,
}

impl Database {
    pub fn delete_progress(&self, episode_id: &str) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM watch_progress WHERE episode_id = ?1", params![episode_id])?;
        Ok(())
    }

    pub fn delete_series_progress(&self, episode_ids: &[String]) -> Result<(), rusqlite::Error> {
        if episode_ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        let placeholders: Vec<String> = episode_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let sql = format!(
            "DELETE FROM watch_progress WHERE episode_id IN ({})",
            placeholders.join(", ")
        );
        let params: Vec<&dyn rusqlite::types::ToSql> =
            episode_ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
        conn.execute(&sql, params.as_slice())?;
        Ok(())
    }

    pub fn get_series_episode_metadata(&self, series_id: &str) -> Vec<EpisodeMetadata> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT episode_id, series_id, tmdb_episode_id, season_number, episode_number,
                        title, overview, air_date, runtime_minutes, still_url
                 FROM episode_metadata WHERE series_id = ?1
                 ORDER BY season_number, episode_number",
            )
            .unwrap();

        stmt.query_map(params![series_id], |row| {
            Ok(EpisodeMetadata {
                episode_id: row.get(0)?,
                series_id: row.get(1)?,
                tmdb_episode_id: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                season_number: row.get::<_, Option<i64>>(3)?.map(|v| v as u32),
                episode_number: row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                title: row.get(5)?,
                overview: row.get(6)?,
                air_date: row.get(7)?,
                runtime_minutes: row.get::<_, Option<i64>>(8)?.map(|v| v as u32),
                still_url: row.get(9)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn get_episode_credits(&self, series_id: &str, season: u32, episode: u32) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT credits_json FROM episode_credits
             WHERE series_id = ?1 AND season_number = ?2 AND episode_number = ?3",
            params![series_id, season as i64, episode as i64],
            |row| row.get(0),
        )
        .ok()
    }

    pub fn save_episode_credits(
        &self,
        series_id: &str,
        season: u32,
        episode: u32,
        credits_json: &str,
    ) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO episode_credits (series_id, season_number, episode_number, credits_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(series_id, season_number, episode_number) DO UPDATE SET
                credits_json = ?4, updated_at = datetime('now')",
            params![series_id, season as i64, episode as i64, credits_json],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_db() -> (TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::new(dir.path()).unwrap();
        (dir, db)
    }

    #[test]
    fn create_database() {
        let (dir, _db) = make_db();
        assert!(dir.path().join("cast.db").exists());
    }

    #[test]
    fn get_progress_returns_none_for_unknown() {
        let (_dir, db) = make_db();
        assert!(db.get_progress("nonexistent").is_none());
    }

    #[test]
    fn update_and_get_progress() {
        let (_dir, db) = make_db();
        db.update_progress("ep1", 120.0, 3600.0).unwrap();

        let p = db.get_progress("ep1").unwrap();
        assert_eq!(p.episode_id, "ep1");
        assert!((p.position_secs - 120.0).abs() < f64::EPSILON);
        assert!((p.duration_secs - 3600.0).abs() < f64::EPSILON);
        assert!(!p.completed);
    }

    #[test]
    fn completed_at_90_percent() {
        let (_dir, db) = make_db();
        // Exactly 90% => completed
        db.update_progress("ep1", 900.0, 1000.0).unwrap();
        let p = db.get_progress("ep1").unwrap();
        assert!(p.completed);
    }

    #[test]
    fn not_completed_below_90_percent() {
        let (_dir, db) = make_db();
        // 89% => not completed
        db.update_progress("ep1", 890.0, 1000.0).unwrap();
        let p = db.get_progress("ep1").unwrap();
        assert!(!p.completed);
    }

    #[test]
    fn completed_above_90_percent() {
        let (_dir, db) = make_db();
        db.update_progress("ep1", 950.0, 1000.0).unwrap();
        let p = db.get_progress("ep1").unwrap();
        assert!(p.completed);
    }

    #[test]
    fn zero_duration_is_not_completed() {
        let (_dir, db) = make_db();
        db.update_progress("ep1", 100.0, 0.0).unwrap();
        let p = db.get_progress("ep1").unwrap();
        assert!(!p.completed);
    }

    #[test]
    fn get_all_progress_empty() {
        let (_dir, db) = make_db();
        assert!(db.get_all_progress().is_empty());
    }

    #[test]
    fn get_all_progress_returns_all() {
        let (_dir, db) = make_db();
        db.update_progress("ep1", 10.0, 100.0).unwrap();
        db.update_progress("ep2", 20.0, 200.0).unwrap();
        db.update_progress("ep3", 30.0, 300.0).unwrap();

        let all = db.get_all_progress();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn get_series_progress_filters() {
        let (_dir, db) = make_db();
        db.update_progress("ep1", 10.0, 100.0).unwrap();
        db.update_progress("ep2", 20.0, 200.0).unwrap();
        db.update_progress("ep3", 30.0, 300.0).unwrap();

        let ids = vec!["ep1".to_string(), "ep3".to_string()];
        let progress = db.get_series_progress(&ids);
        assert_eq!(progress.len(), 2);
        let ep_ids: Vec<&str> = progress.iter().map(|p| p.episode_id.as_str()).collect();
        assert!(ep_ids.contains(&"ep1"));
        assert!(ep_ids.contains(&"ep3"));
        assert!(!ep_ids.contains(&"ep2"));
    }

    #[test]
    fn get_series_progress_empty_ids() {
        let (_dir, db) = make_db();
        db.update_progress("ep1", 10.0, 100.0).unwrap();

        let ids: Vec<String> = vec![];
        let progress = db.get_series_progress(&ids);
        assert!(progress.is_empty());
    }

    #[test]
    fn update_progress_overwrites_old_value() {
        let (_dir, db) = make_db();
        db.update_progress("ep1", 100.0, 1000.0).unwrap();
        let p1 = db.get_progress("ep1").unwrap();
        assert!((p1.position_secs - 100.0).abs() < f64::EPSILON);

        db.update_progress("ep1", 500.0, 1000.0).unwrap();
        let p2 = db.get_progress("ep1").unwrap();
        assert!((p2.position_secs - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn update_progress_can_change_completed_status() {
        let (_dir, db) = make_db();
        // Start not completed
        db.update_progress("ep1", 100.0, 1000.0).unwrap();
        assert!(!db.get_progress("ep1").unwrap().completed);

        // Now mark as completed
        db.update_progress("ep1", 950.0, 1000.0).unwrap();
        assert!(db.get_progress("ep1").unwrap().completed);
    }

    #[test]
    fn delete_progress_removes_entry() {
        let (_dir, db) = make_db();
        db.update_progress("ep1", 120.0, 3600.0).unwrap();
        assert!(db.get_progress("ep1").is_some());

        db.delete_progress("ep1").unwrap();
        assert!(db.get_progress("ep1").is_none());
    }

    #[test]
    fn delete_progress_noop_for_unknown() {
        let (_dir, db) = make_db();
        // Should not error even if episode has no progress
        db.delete_progress("nonexistent").unwrap();
    }

    #[test]
    fn delete_series_progress_removes_all() {
        let (_dir, db) = make_db();
        db.update_progress("ep1", 10.0, 100.0).unwrap();
        db.update_progress("ep2", 20.0, 200.0).unwrap();
        db.update_progress("ep3", 30.0, 300.0).unwrap();

        let ids = vec!["ep1".to_string(), "ep2".to_string(), "ep3".to_string()];
        db.delete_series_progress(&ids).unwrap();

        assert!(db.get_progress("ep1").is_none());
        assert!(db.get_progress("ep2").is_none());
        assert!(db.get_progress("ep3").is_none());
    }

    #[test]
    fn delete_series_progress_empty_ids() {
        let (_dir, db) = make_db();
        db.update_progress("ep1", 10.0, 100.0).unwrap();

        let ids: Vec<String> = vec![];
        db.delete_series_progress(&ids).unwrap();

        // ep1 should still exist
        assert!(db.get_progress("ep1").is_some());
    }
}
