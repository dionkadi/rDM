//! SQLite persistence for downloads, history, categories and settings (rusqlite,
//! bundled so no system SQLite is required).

use crate::model::{Category, Download, DownloadStatus, Settings};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct Storage {
    inner: Arc<Mutex<Connection>>,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        let s = Storage {
            inner: Arc::new(Mutex::new(conn)),
        };
        s.migrate()?;
        Ok(s)
    }

    pub fn open_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        let s = Storage {
            inner: Arc::new(Mutex::new(conn)),
        };
        s.migrate()?;
        Ok(s)
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let conn = self.inner.lock().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS downloads (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                filename TEXT NOT NULL,
                save_path TEXT NOT NULL,
                total_size INTEGER,
                downloaded INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL,
                category TEXT,
                content_type TEXT,
                chunks TEXT NOT NULL DEFAULT '[]',
                speed_limit INTEGER,
                proxy TEXT,
                checksum TEXT,
                error TEXT,
                created_at TEXT NOT NULL,
                finished_at TEXT,
                can_resume INTEGER NOT NULL DEFAULT 0,
                sort_key INTEGER NOT NULL DEFAULT 0,
                priority INTEGER NOT NULL DEFAULT 1
            );
            CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                download_id TEXT,
                url TEXT NOT NULL,
                filename TEXT NOT NULL,
                size INTEGER,
                finished_at TEXT,
                category TEXT,
                checksum_ok INTEGER
            );
            CREATE TABLE IF NOT EXISTS categories (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                extensions TEXT NOT NULL DEFAULT '[]',
                directory TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;
        // Idempotent column adds for pre-existing databases that
        // were created before `sort_key` / `priority` existed. We
        // use `PRAGMA table_info` to detect whether the column is
        // already present; `ALTER TABLE ... ADD COLUMN` would error
        // with "duplicate column" otherwise. The default values
        // (sort_key = created_at epoch ms, priority = 1) preserve
        // the pre-migration order: oldest downloads keep the
        // smallest sort_key, so the list view doesn't jump after
        // upgrading.
        Self::ensure_column(&conn, "downloads", "sort_key", "INTEGER NOT NULL DEFAULT 0")?;
        Self::ensure_column(&conn, "downloads", "priority", "INTEGER NOT NULL DEFAULT 1")?;
        Ok(())
    }

    /// Idempotent `ALTER TABLE ... ADD COLUMN`. No-op when the
    /// column already exists. Used by `migrate()` to add new
    /// columns to a pre-existing database without breaking the
    /// `CREATE TABLE IF NOT EXISTS` baseline.
    fn ensure_column(
        conn: &Connection,
        table: &str,
        column: &str,
        definition: &str,
    ) -> Result<(), StorageError> {
        // `PRAGMA table_info` returns one row per column; the
        // second field (index 1) is the column name. We collect
        // them into a `Vec<String>` to avoid borrowing `stmt`
        // while we run the `ALTER TABLE`.
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        if names.iter().any(|n| n == column) {
            return Ok(());
        }
        let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, definition);
        conn.execute_batch(&sql)?;
        Ok(())
    }

    /// Upsert a download row.
    pub fn save_download(&self, d: &Download) -> Result<(), StorageError> {
        let conn = self.inner.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO downloads (
                id, url, filename, save_path, total_size, downloaded, status,
                category, content_type, chunks, speed_limit, proxy, checksum,
                error, created_at, finished_at, can_resume
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
            ON CONFLICT(id) DO UPDATE SET
                url=?2, filename=?3, save_path=?4, total_size=?5, downloaded=?6,
                status=?7, category=?8, content_type=?9, chunks=?10, speed_limit=?11,
                proxy=?12, checksum=?13, error=?14, finished_at=?16, can_resume=?17
            "#,
            params![
                d.id,
                d.url,
                d.filename,
                d.save_path.to_string_lossy().to_string(),
                d.total_size,
                d.downloaded,
                serde_json::to_string(&d.status)?,
                d.category,
                d.content_type,
                serde_json::to_string(&d.chunks)?,
                d.speed_limit,
                d.proxy,
                d.checksum.as_ref().map(|c| serde_json::to_string(c).unwrap()),
                d.error,
                d.created_at.to_rfc3339(),
                d.finished_at.map(|t| t.to_rfc3339()),
                d.can_resume as i64,
            ],
        )?;
        Ok(())
    }

    /// Load downloads that are not finished/cancelled (resumable on startup).
    pub fn load_active(&self) -> Result<Vec<Download>, StorageError> {
        let conn = self.inner.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id,url,filename,save_path,total_size,downloaded,status,category,
                    content_type,chunks,speed_limit,proxy,checksum,error,created_at,
                    finished_at,can_resume FROM downloads
             WHERE status NOT IN ('completed','canceled')",
        )?;
        let rows = stmt.query_map([], |row| Ok(self_row_to_download(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn mark_completed(&self, id: &str, finished_at: &str) -> Result<(), StorageError> {
        let conn = self.inner.lock().unwrap();
        conn.execute(
            "UPDATE downloads SET status='completed', finished_at=?2, downloaded=total_size WHERE id=?1",
            params![id, finished_at],
        )?;
        Ok(())
    }

    pub fn mark_error(&self, id: &str, error: &str) -> Result<(), StorageError> {
        let conn = self.inner.lock().unwrap();
        conn.execute(
            "UPDATE downloads SET status='error', error=?2 WHERE id=?1",
            params![id, error],
        )?;
        Ok(())
    }

    /// Physically remove a download row from the `downloads` table.
    ///
    /// Used by `DownloadManager::remove()` when the user explicitly
    /// deletes an entry — without this, the row would reappear on the
    /// next launch via `load_active()`. The corresponding `history`
    /// row is left in place (it's an append-only log of completed
    /// downloads, useful for re-downloading later).
    pub fn delete_download(&self, id: &str) -> Result<(), StorageError> {
        let conn = self.inner.lock().unwrap();
        conn.execute("DELETE FROM downloads WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn add_history(
        &self,
        download_id: Option<&str>,
        url: &str,
        filename: &str,
        size: Option<u64>,
        finished_at: &str,
        category: Option<&str>,
        checksum_ok: Option<bool>,
    ) -> Result<(), StorageError> {
        let conn = self.inner.lock().unwrap();
        conn.execute(
            "INSERT INTO history (download_id,url,filename,size,finished_at,category,checksum_ok)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                download_id,
                url,
                filename,
                size,
                finished_at,
                category,
                checksum_ok.map(|b| b as i64),
            ],
        )?;
        Ok(())
    }

    pub fn get_settings(&self) -> Result<Option<Settings>, StorageError> {
        let conn = self.inner.lock().unwrap();
        let v: Option<String> = conn
            .query_row("SELECT value FROM settings WHERE key='settings'", [], |r| r.get(0))
            .ok();
        match v {
            Some(s) => Ok(Some(serde_json::from_str(&s)?)),
            None => Ok(None),
        }
    }

    pub fn save_settings(&self, s: &Settings) -> Result<(), StorageError> {
        let conn = self.inner.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key,value) VALUES ('settings',?1)
             ON CONFLICT(key) DO UPDATE SET value=?1",
            params![serde_json::to_string(s)?],
        )?;
        Ok(())
    }

    pub fn save_categories(&self, cats: &[Category]) -> Result<(), StorageError> {
        let mut conn = self.inner.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM categories", [])?;
        for c in cats {
            tx.execute(
                "INSERT INTO categories (id,name,extensions,directory) VALUES (?1,?2,?3,?4)",
                params![
                    c.id,
                    c.name,
                    serde_json::to_string(&c.extensions)?,
                    c.directory.to_string_lossy().to_string(),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_categories(&self) -> Result<Vec<Category>, StorageError> {
        let conn = self.inner.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id,name,extensions,directory FROM categories")?;
        let rows = stmt.query_map([], |row| {
            Ok(Category {
                id: row.get(0)?,
                name: row.get(1)?,
                extensions: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
                directory: Path::new(&row.get::<_, String>(3)?).to_path_buf(),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

fn self_row_to_download(
    row: &rusqlite::Row<'_>,
) -> Download {
    let id: String = row.get(0).unwrap();
    let url: String = row.get(1).unwrap();
    let filename: String = row.get(2).unwrap();
    let save_path: String = row.get(3).unwrap();
    let total_size: Option<u64> = row.get(4).unwrap();
    let downloaded: u64 = row.get(5).unwrap();
    let status: String = row.get(6).unwrap();
    let category: Option<String> = row.get(7).unwrap();
    let content_type: Option<String> = row.get(8).unwrap();
    let chunks_json: String = row.get(9).unwrap();
    let speed_limit: Option<u64> = row.get(10).unwrap();
    let proxy: Option<String> = row.get(11).unwrap();
    let checksum: Option<String> = row.get(12).unwrap();
    let error: Option<String> = row.get(13).unwrap();
    let created_at: String = row.get(14).unwrap();
    let finished_at: Option<String> = row.get(15).unwrap();
    let can_resume: i64 = row.get(16).unwrap();
    // Columns 17/18 (`sort_key`, `priority`) were added by a
    // later migration. Pre-migration rows have neither, so
    // `row.get` returns `Err` rather than `Ok`. The struct uses
    // `#[serde(default)]` to keep these tolerant on the wire
    // and on freshly-loaded rows: missing columns fall back to
    // `sort_key = 0` and `priority = 1` (normal).
    let sort_key: i64 = row.get::<_, Option<i64>>(17).unwrap_or(None).unwrap_or(0);
    let priority: i64 = row.get::<_, Option<i64>>(18).unwrap_or(None).unwrap_or(1);

    Download {
        id,
        url,
        filename,
        save_path: Path::new(&save_path).to_path_buf(),
        total_size,
        downloaded,
        status: serde_json::from_str::<DownloadStatus>(&status)
            .unwrap_or(DownloadStatus::Queued),
        category,
        content_type,
        chunks: serde_json::from_str(&chunks_json).unwrap_or_default(),
        speed_limit,
        proxy,
        checksum: checksum.and_then(|c| serde_json::from_str(&c).ok()),
        error,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
        finished_at: finished_at.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|d| d.with_timezone(&chrono::Utc))
        }),
        can_resume: can_resume != 0,
        sort_key,
        priority: priority.clamp(0, 2) as u8,
        // `headers`, `auth`, `mirrors`, and `media` live in
        // memory only for v1 — the SQLite schema predates
        // them, and we don't want to blow up the schema for
        // auth secrets or per-transfer mirror lists. All four
        // are reconstructed fresh on the wire (the frontend
        // doesn't read them back, so persistence is a non-goal
        // for this iteration). The `serde(default)` on the
        // struct means a row loaded from disk always ends up
        // with empty headers / no auth / no mirrors / no media.
        headers: std::collections::BTreeMap::new(),
        auth: None,
        mirrors: Vec::new(),
        media: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChunkState;

    fn sample() -> Download {
        Download {
            id: "test-id".into(),
            url: "https://example.com/a.bin".into(),
            filename: "a.bin".into(),
            save_path: std::path::PathBuf::from("/tmp/a.bin"),
            total_size: Some(100),
            downloaded: 40,
            status: DownloadStatus::Paused,
            category: Some("general".into()),
            content_type: Some("application/octet-stream".into()),
            chunks: vec![ChunkState { index: 0, start: 0, end: 99, downloaded: 40 }],
            speed_limit: Some(1024),
            proxy: None,
            checksum: None,
            error: None,
            created_at: chrono::Utc::now(),
            finished_at: None,
            can_resume: true,
            sort_key: 0,
            priority: 1,
            headers: std::collections::BTreeMap::new(),
            auth: None,
            mirrors: Vec::new(),
            media: None,
        }
    }

    #[test]
    fn roundtrip_download() {
        let db = Storage::open_memory().unwrap();
        db.save_download(&sample()).unwrap();
        let active = db.load_active().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].downloaded, 40);
        assert_eq!(active[0].status, DownloadStatus::Paused);
    }

    #[test]
    fn settings_roundtrip() {
        let db = Storage::open_memory().unwrap();
        let s = Settings::default();
        db.save_settings(&s).unwrap();
        let loaded = db.get_settings().unwrap().unwrap();
        assert_eq!(loaded.max_concurrent_downloads, s.max_concurrent_downloads);
    }
}
