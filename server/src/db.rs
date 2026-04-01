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

impl Database {
    pub fn new(media_path: &Path) -> Result<Self, rusqlite::Error> {
        let db_path = media_path.join(".cast.db");
        let conn = Connection::open(db_path)?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS watch_progress (
                episode_id TEXT PRIMARY KEY,
                position_secs REAL NOT NULL DEFAULT 0,
                duration_secs REAL NOT NULL DEFAULT 0,
                completed INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
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
        assert!(dir.path().join(".cast.db").exists());
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
}
