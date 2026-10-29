// SPDX-License-Identifier: GPL-3.0-or-later

//! SQLite storage: schema, migrations, downloads, segments and settings.
//!
//! One database file holds every download and the progress of each of its segments.
//! Segment progress is saved in a single transaction per checkpoint, so after a crash
//! the stored progress is always a state that really existed on disk.

use std::fmt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use omnidm_core::engine::Method;
use omnidm_core::{Segment, SegmentId};
pub use omnidm_queue::PostAction;
pub use omnidm_queue::category::CategoryRule;
pub use omnidm_queue::schedule::Schedule;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

pub mod checkpoint;
#[cfg(windows)]
mod dpapi;

/// Schema version this build creates and understands.
pub const SCHEMA_VERSION: u32 = 5;

/// Each entry upgrades the schema from version `index` to `index + 1`.
const MIGRATIONS: [&str; SCHEMA_VERSION as usize] = [
    r#"
    CREATE TABLE downloads (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        url           TEXT    NOT NULL,
        final_url     TEXT,
        save_dir      TEXT    NOT NULL,
        file_name     TEXT,
        referrer      TEXT,
        user_agent    TEXT,
        total_size    INTEGER,
        etag          TEXT,
        last_modified TEXT,
        resumable     INTEGER NOT NULL DEFAULT 0,
        status        TEXT    NOT NULL,
        error         TEXT,
        created_at    INTEGER NOT NULL,
        completed_at  INTEGER
    );
    CREATE TABLE download_headers (
        download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
        position    INTEGER NOT NULL,
        name        TEXT    NOT NULL,
        value       TEXT    NOT NULL,
        PRIMARY KEY (download_id, position)
    );
    CREATE TABLE segments (
        download_id INTEGER NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
        segment_id  INTEGER NOT NULL,
        start       INTEGER NOT NULL,
        "end"       INTEGER NOT NULL,
        written     INTEGER NOT NULL,
        PRIMARY KEY (download_id, segment_id)
    );
"#,
    // 2: POST downloads. Downloads probed before this were all GET.
    r#"
    ALTER TABLE downloads ADD COLUMN method TEXT NOT NULL DEFAULT 'GET';
    ALTER TABLE downloads ADD COLUMN body BLOB;
    ALTER TABLE downloads ADD COLUMN final_method TEXT;
    UPDATE downloads SET final_method = 'GET' WHERE final_url IS NOT NULL;
"#,
    // 3: cookies of unfinished downloads, encrypted for the current Windows user.
    r#"
    CREATE TABLE secrets (
        download_id INTEGER PRIMARY KEY REFERENCES downloads(id) ON DELETE CASCADE,
        cookies     BLOB    NOT NULL
    );
"#,
    // 4: queues and categories. The Main queue always exists; the default categories are
    // added by `seed_categories`.
    r#"
    CREATE TABLE queues (
        id             INTEGER PRIMARY KEY AUTOINCREMENT,
        name           TEXT    NOT NULL UNIQUE,
        max_concurrent INTEGER NOT NULL,
        schedule_json  TEXT    NOT NULL DEFAULT '{}',
        on_complete    TEXT    NOT NULL DEFAULT 'none'
    );
    INSERT INTO queues (id, name, max_concurrent) VALUES (1, 'Main', 3);
    CREATE TABLE queue_items (
        download_id INTEGER PRIMARY KEY REFERENCES downloads(id) ON DELETE CASCADE,
        queue_id    INTEGER NOT NULL REFERENCES queues(id) ON DELETE CASCADE,
        position    INTEGER NOT NULL
    );
    CREATE INDEX queue_items_order ON queue_items (queue_id, position);
    CREATE TABLE categories (
        id        INTEGER PRIMARY KEY AUTOINCREMENT,
        name      TEXT    NOT NULL UNIQUE,
        position  INTEGER NOT NULL,
        rule_json TEXT    NOT NULL,
        save_dir  TEXT
    );
    ALTER TABLE downloads ADD COLUMN category_id INTEGER
        REFERENCES categories(id) ON DELETE SET NULL;
"#,
    r#"
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
"#,
];

const DOWNLOAD_COLUMNS: &str = "id, url, save_dir, file_name, referrer, user_agent, final_url, \
     total_size, etag, last_modified, resumable, status, error, created_at, completed_at, \
     method, body, final_method, category_id";

/// The queue every database starts with. It can't be deleted.
pub const MAIN_QUEUE: QueueId = QueueId(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QueueId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CategoryId(pub i64);

/// A queue's settings: what the user can change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSettings {
    pub name: String,
    /// Downloads of this queue running at once.
    pub max_concurrent: u32,
    pub schedule: Schedule,
    pub on_complete: PostAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRecord {
    pub id: QueueId,
    pub settings: QueueSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategorySettings {
    pub name: String,
    pub rule: CategoryRule,
    /// Where its downloads are saved; `None` means the general downloads folder.
    pub save_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryRecord {
    pub id: CategoryId,
    pub settings: CategorySettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DownloadId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Queued,
    Probing,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl Status {
    /// The name stored in the database: `queued`, `running`, `completed`…
    pub fn as_str(self) -> &'static str {
        status_name(self)
    }
}

/// What a caller provides to add a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDownload {
    pub url: String,
    pub save_dir: String,
    /// Chosen by the user; otherwise decided after the probe.
    pub file_name: Option<String>,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
    /// Extra request headers as `(name, value)` pairs.
    pub headers: Vec<(String, String)>,
    pub method: Method,
    /// The form data of a `POST`; empty for a `GET`.
    pub body: Vec<u8>,
}

/// What the probe learned about the remote file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    pub final_url: String,
    pub file_name: String,
    pub total_size: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub resumable: bool,
    /// The method after redirects.
    pub final_method: Method,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRecord {
    pub id: DownloadId,
    pub url: String,
    pub save_dir: String,
    pub file_name: Option<String>,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
    pub headers: Vec<(String, String)>,
    pub method: Method,
    pub body: Vec<u8>,
    pub final_url: Option<String>,
    /// The method after redirects; `None` until probed.
    pub final_method: Option<Method>,
    pub total_size: Option<u64>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub resumable: bool,
    pub status: Status,
    pub error: Option<String>,
    /// Unix seconds.
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub category: Option<CategoryId>,
}

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    /// The database was written by a newer OmniDm.
    TooNew {
        found: u32,
        supported: u32,
    },
    /// A stored value can't be read back (e.g. an unknown status).
    Corrupt(String),
    /// A size or offset is too large to store.
    OutOfRange(u64),
    /// Not available on this operating system.
    Unsupported(&'static str),
    /// The change is never allowed, e.g. deleting the Main queue.
    NotAllowed(&'static str),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "database error: {e}"),
            Self::TooNew { found, supported } => write!(
                f,
                "database version {found} is newer than this OmniDm supports ({supported})"
            ),
            Self::Corrupt(what) => write!(f, "database contains invalid data: {what}"),
            Self::OutOfRange(v) => write!(f, "value {v} is too large to store"),
            Self::Unsupported(what) => write!(f, "{what} is not supported on this system"),
            Self::NotAllowed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens (or creates) the database file and brings its schema up to date.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        // WAL keeps the database consistent across crashes and lets readers run during
        // checkpoints. NORMAL sync may lose the newest checkpoint on power loss, which only
        // rolls progress back to an earlier, still valid state.
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    /// A private in-memory database, for tests and dry runs.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn insert_download(&mut self, new: &NewDownload) -> Result<DownloadId> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO downloads (url, save_dir, file_name, referrer, user_agent, status,
                                    created_at, method, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                new.url,
                new.save_dir,
                new.file_name,
                new.referrer,
                new.user_agent,
                status_name(Status::Queued),
                unix_now(),
                new.method.as_str(),
                (!new.body.is_empty()).then_some(&new.body),
            ],
        )?;
        let id = tx.last_insert_rowid();
        for (position, (name, value)) in new.headers.iter().enumerate() {
            tx.execute(
                "INSERT INTO download_headers (download_id, position, name, value)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, position as i64, name, value],
            )?;
        }
        tx.commit()?;
        Ok(DownloadId(id))
    }

    pub fn get_download(&self, id: DownloadId) -> Result<Option<DownloadRecord>> {
        let sql = format!("SELECT {DOWNLOAD_COLUMNS} FROM downloads WHERE id = ?1");
        let row = self
            .conn
            .query_row(&sql, [id.0], read_download)
            .optional()?;
        match row {
            Some(record) => Ok(Some(self.with_headers(record?)?)),
            None => Ok(None),
        }
    }

    /// Every download, oldest first.
    pub fn list_downloads(&self) -> Result<Vec<DownloadRecord>> {
        let sql = format!("SELECT {DOWNLOAD_COLUMNS} FROM downloads ORDER BY id");
        let mut stmt = self.conn.prepare(&sql)?;
        let records = stmt
            .query_map([], read_download)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        records
            .into_iter()
            .map(|record| self.with_headers(record?))
            .collect()
    }

    /// The newest download of `url` into `save_dir` that is neither completed nor cancelled.
    pub fn find_unfinished(&self, url: &str, save_dir: &str) -> Result<Option<DownloadRecord>> {
        let sql = format!(
            "SELECT {DOWNLOAD_COLUMNS} FROM downloads
             WHERE url = ?1 AND save_dir = ?2 AND status NOT IN (?3, ?4)
             ORDER BY id DESC LIMIT 1"
        );
        let row = self
            .conn
            .query_row(
                &sql,
                params![
                    url,
                    save_dir,
                    status_name(Status::Completed),
                    status_name(Status::Cancelled)
                ],
                read_download,
            )
            .optional()?;
        match row {
            Some(record) => Ok(Some(self.with_headers(record?)?)),
            None => Ok(None),
        }
    }

    pub fn record_probe(&mut self, id: DownloadId, probe: &ProbeResult) -> Result<()> {
        let total_size = probe.total_size.map(to_i64).transpose()?;
        self.conn.execute(
            "UPDATE downloads
             SET final_url = ?2, file_name = ?3, total_size = ?4, etag = ?5,
                 last_modified = ?6, resumable = ?7, final_method = ?8
             WHERE id = ?1",
            params![
                id.0,
                probe.final_url,
                probe.file_name,
                total_size,
                probe.etag,
                probe.last_modified,
                probe.resumable,
                probe.final_method.as_str(),
            ],
        )?;
        Ok(())
    }

    /// Sets the status; `error` is stored for `Failed` and cleared otherwise.
    pub fn set_status(
        &mut self,
        id: DownloadId,
        status: Status,
        error: Option<&str>,
    ) -> Result<()> {
        let error = error.filter(|_| status == Status::Failed);
        let completed_at = (status == Status::Completed).then(unix_now);
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE downloads SET status = ?2, error = ?3, completed_at = ?4 WHERE id = ?1",
            params![id.0, status_name(status), error, completed_at],
        )?;
        // A finished download needs its cookies no more.
        if status == Status::Completed {
            tx.execute("DELETE FROM secrets WHERE download_id = ?1", [id.0])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Records the name the file was finally saved under (it may differ from the planned
    /// one when that was taken).
    pub fn set_file_name(&mut self, id: DownloadId, file_name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE downloads SET file_name = ?2 WHERE id = ?1",
            params![id.0, file_name],
        )?;
        Ok(())
    }

    /// Moves a download to another folder (before it has started writing there).
    pub fn set_save_dir(&mut self, id: DownloadId, save_dir: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE downloads SET save_dir = ?2 WHERE id = ?1",
            params![id.0, save_dir],
        )?;
        Ok(())
    }

    /// A setting's saved value; `None` when it was never set.
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Saves a download's cookies (any serialised form), encrypted for the current
    /// Windows user. They are deleted when the download completes or is deleted.
    pub fn save_cookies(&mut self, id: DownloadId, cookies: &[u8]) -> Result<()> {
        let sealed = seal(cookies)?;
        self.conn.execute(
            "INSERT INTO secrets (download_id, cookies) VALUES (?1, ?2)
             ON CONFLICT (download_id) DO UPDATE SET cookies = excluded.cookies",
            params![id.0, sealed],
        )?;
        Ok(())
    }

    /// The cookies saved with [`save_cookies`](Self::save_cookies), decrypted.
    pub fn load_cookies(&self, id: DownloadId) -> Result<Option<Vec<u8>>> {
        let sealed: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT cookies FROM secrets WHERE download_id = ?1",
                [id.0],
                |r| r.get(0),
            )
            .optional()?;
        sealed.map(|sealed| unseal(&sealed)).transpose()
    }

    /// Replaces the stored segments of a download in one transaction.
    pub fn save_segments(&mut self, id: DownloadId, segments: &[Segment]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM segments WHERE download_id = ?1", [id.0])?;
        {
            let mut insert = tx.prepare_cached(
                r#"INSERT INTO segments (download_id, segment_id, start, "end", written)
                   VALUES (?1, ?2, ?3, ?4, ?5)"#,
            )?;
            for s in segments {
                insert.execute(params![
                    id.0,
                    s.id.0,
                    to_i64(s.start)?,
                    to_i64(s.end)?,
                    to_i64(s.written)?,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The stored segments of a download, ordered by start offset.
    pub fn load_segments(&self, id: DownloadId) -> Result<Vec<Segment>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT segment_id, start, "end", written FROM segments
               WHERE download_id = ?1 ORDER BY start"#,
        )?;
        let rows = stmt
            .query_map([id.0], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(segment_id, start, end, written)| {
                Ok(Segment {
                    id: SegmentId(segment_id),
                    start: to_u64(start)?,
                    end: to_u64(end)?,
                    written: to_u64(written)?,
                })
            })
            .collect()
    }

    /// Deletes a download and its segments.
    pub fn delete_download(&mut self, id: DownloadId) -> Result<()> {
        self.conn
            .execute("DELETE FROM downloads WHERE id = ?1", [id.0])?;
        Ok(())
    }

    /// Every queue, Main first, then in the order they were made.
    pub fn list_queues(&self) -> Result<Vec<QueueRecord>> {
        let sql = format!("SELECT {QUEUE_COLUMNS} FROM queues ORDER BY id");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map([], read_queue)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().collect()
    }

    pub fn get_queue(&self, id: QueueId) -> Result<Option<QueueRecord>> {
        let sql = format!("SELECT {QUEUE_COLUMNS} FROM queues WHERE id = ?1");
        self.conn
            .query_row(&sql, [id.0], read_queue)
            .optional()?
            .transpose()
    }

    pub fn insert_queue(&mut self, settings: &QueueSettings) -> Result<QueueId> {
        self.conn.execute(
            "INSERT INTO queues (name, max_concurrent, schedule_json, on_complete)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                settings.name,
                settings.max_concurrent,
                schedule_json(&settings.schedule)?,
                settings.on_complete.as_str(),
            ],
        )?;
        Ok(QueueId(self.conn.last_insert_rowid()))
    }

    pub fn update_queue(&mut self, id: QueueId, settings: &QueueSettings) -> Result<()> {
        self.conn.execute(
            "UPDATE queues SET name = ?2, max_concurrent = ?3, schedule_json = ?4,
                               on_complete = ?5
             WHERE id = ?1",
            params![
                id.0,
                settings.name,
                settings.max_concurrent,
                schedule_json(&settings.schedule)?,
                settings.on_complete.as_str(),
            ],
        )?;
        Ok(())
    }

    /// Deletes a queue. Its downloads stay, in no queue.
    pub fn delete_queue(&mut self, id: QueueId) -> Result<()> {
        if id == MAIN_QUEUE {
            return Err(StoreError::NotAllowed("the Main queue can't be deleted"));
        }
        self.conn
            .execute("DELETE FROM queues WHERE id = ?1", [id.0])?;
        Ok(())
    }

    /// The downloads of a queue, in order.
    pub fn queue_downloads(&self, id: QueueId) -> Result<Vec<DownloadId>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT download_id FROM queue_items WHERE queue_id = ?1 ORDER BY position",
        )?;
        let ids = stmt
            .query_map([id.0], |r| Ok(DownloadId(r.get(0)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    /// The queue a download is in, if any.
    pub fn queue_of(&self, download: DownloadId) -> Result<Option<QueueId>> {
        Ok(self
            .conn
            .query_row(
                "SELECT queue_id FROM queue_items WHERE download_id = ?1",
                [download.0],
                |r| Ok(QueueId(r.get(0)?)),
            )
            .optional()?)
    }

    /// Puts a download at the end of `queue`, taking it out of any other queue. A download
    /// already in `queue` keeps its place.
    pub fn add_to_queue(&mut self, queue: QueueId, download: DownloadId) -> Result<()> {
        self.conn.execute(
            "INSERT INTO queue_items (download_id, queue_id, position)
             VALUES (?1, ?2,
                     (SELECT COALESCE(MAX(position), -1) + 1 FROM queue_items WHERE queue_id = ?2))
             ON CONFLICT (download_id) DO UPDATE
                 SET queue_id = excluded.queue_id, position = excluded.position
                 WHERE queue_id <> excluded.queue_id",
            params![download.0, queue.0],
        )?;
        Ok(())
    }

    pub fn remove_from_queue(&mut self, download: DownloadId) -> Result<()> {
        self.conn.execute(
            "DELETE FROM queue_items WHERE download_id = ?1",
            [download.0],
        )?;
        Ok(())
    }

    /// Moves a download to `index` in its queue (past the end means last). `false` if it
    /// isn't in a queue.
    pub fn move_in_queue(&mut self, download: DownloadId, index: usize) -> Result<bool> {
        let Some(queue) = self.queue_of(download)? else {
            return Ok(false);
        };
        let mut order = self.queue_downloads(queue)?;
        order.retain(|&id| id != download);
        order.insert(index.min(order.len()), download);
        let tx = self.conn.transaction()?;
        renumber(
            &tx,
            "UPDATE queue_items SET position = ?2 WHERE download_id = ?1",
            order.iter().map(|id| id.0),
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Every category, in the order they are tried.
    pub fn list_categories(&self) -> Result<Vec<CategoryRecord>> {
        let sql = format!("SELECT {CATEGORY_COLUMNS} FROM categories ORDER BY position, id");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map([], read_category)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().collect()
    }

    pub fn get_category(&self, id: CategoryId) -> Result<Option<CategoryRecord>> {
        let sql = format!("SELECT {CATEGORY_COLUMNS} FROM categories WHERE id = ?1");
        self.conn
            .query_row(&sql, [id.0], read_category)
            .optional()?
            .transpose()
    }

    /// Adds a category, tried after the existing ones.
    pub fn insert_category(&mut self, settings: &CategorySettings) -> Result<CategoryId> {
        insert_category(&self.conn, settings)
    }

    pub fn update_category(&mut self, id: CategoryId, settings: &CategorySettings) -> Result<()> {
        self.conn.execute(
            "UPDATE categories SET name = ?2, rule_json = ?3, save_dir = ?4 WHERE id = ?1",
            params![
                id.0,
                settings.name,
                rule_json(&settings.rule)?,
                settings.save_dir
            ],
        )?;
        Ok(())
    }

    /// Deletes a category. Its downloads stay, with no category.
    pub fn delete_category(&mut self, id: CategoryId) -> Result<()> {
        self.conn
            .execute("DELETE FROM categories WHERE id = ?1", [id.0])?;
        Ok(())
    }

    /// Moves a category to `index` in the order they are tried (past the end means last).
    pub fn move_category(&mut self, id: CategoryId, index: usize) -> Result<()> {
        let mut order: Vec<CategoryId> = self.list_categories()?.iter().map(|c| c.id).collect();
        if !order.contains(&id) {
            return Ok(());
        }
        order.retain(|&c| c != id);
        order.insert(index.min(order.len()), id);
        let tx = self.conn.transaction()?;
        renumber(
            &tx,
            "UPDATE categories SET position = ?2 WHERE id = ?1",
            order.iter().map(|c| c.0),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_download_category(
        &mut self,
        download: DownloadId,
        category: Option<CategoryId>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE downloads SET category_id = ?2 WHERE id = ?1",
            params![download.0, category.map(|c| c.0)],
        )?;
        Ok(())
    }

    fn with_headers(&self, mut record: DownloadRecord) -> Result<DownloadRecord> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT name, value FROM download_headers WHERE download_id = ?1 ORDER BY position",
        )?;
        record.headers = stmt
            .query_map([record.id.0], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(record)
    }
}

#[cfg(windows)]
fn seal(plain: &[u8]) -> Result<Vec<u8>> {
    dpapi::protect(plain).map_err(|e| StoreError::Corrupt(format!("can't encrypt cookies: {e}")))
}

#[cfg(windows)]
fn unseal(sealed: &[u8]) -> Result<Vec<u8>> {
    dpapi::unprotect(sealed)
        .map_err(|e| StoreError::Corrupt(format!("saved cookies can't be decrypted: {e}")))
}

#[cfg(not(windows))]
fn seal(_plain: &[u8]) -> Result<Vec<u8>> {
    Err(StoreError::Unsupported("saving cookies"))
}

#[cfg(not(windows))]
fn unseal(_sealed: &[u8]) -> Result<Vec<u8>> {
    Err(StoreError::Unsupported("saving cookies"))
}

fn parse_method(name: &str) -> Result<Method> {
    Method::from_name(name).ok_or_else(|| StoreError::Corrupt(format!("unknown method {name:?}")))
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let found: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if found > SCHEMA_VERSION {
        return Err(StoreError::TooNew {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    for version in found..SCHEMA_VERSION {
        let tx = conn.transaction()?;
        tx.execute_batch(MIGRATIONS[version as usize])?;
        if version + 1 == 4 {
            seed_categories(&tx)?;
        }
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
    }
    Ok(())
}

/// Reads one `DOWNLOAD_COLUMNS` row. The outer result is SQLite's, the inner one ours,
/// so invalid stored values surface as [`StoreError::Corrupt`] instead of a panic.
fn read_download(r: &Row<'_>) -> rusqlite::Result<Result<DownloadRecord>> {
    let total_size: Option<i64> = r.get(7)?;
    let status: String = r.get(11)?;
    let method: String = r.get(15)?;
    let body: Option<Vec<u8>> = r.get(16)?;
    let final_method: Option<String> = r.get(17)?;
    let category: Option<i64> = r.get(18)?;
    let id = DownloadId(r.get(0)?);
    let record = (|| {
        Ok(DownloadRecord {
            id,
            url: r.get(1)?,
            save_dir: r.get(2)?,
            file_name: r.get(3)?,
            referrer: r.get(4)?,
            user_agent: r.get(5)?,
            headers: Vec::new(),
            method: parse_method(&method)?,
            body: body.unwrap_or_default(),
            final_url: r.get(6)?,
            final_method: final_method.as_deref().map(parse_method).transpose()?,
            total_size: total_size.map(to_u64).transpose()?,
            etag: r.get(8)?,
            last_modified: r.get(9)?,
            resumable: r.get(10)?,
            status: parse_status(&status)?,
            error: r.get(12)?,
            created_at: r.get(13)?,
            completed_at: r.get(14)?,
            category: category.map(CategoryId),
        })
    })();
    Ok(record)
}

const QUEUE_COLUMNS: &str = "id, name, max_concurrent, schedule_json, on_complete";
const CATEGORY_COLUMNS: &str = "id, name, rule_json, save_dir";

/// Reads one `QUEUE_COLUMNS` row (outer result SQLite's, inner ours, as `read_download`).
fn read_queue(r: &Row<'_>) -> rusqlite::Result<Result<QueueRecord>> {
    let id = QueueId(r.get(0)?);
    let name: String = r.get(1)?;
    let max_concurrent: i64 = r.get(2)?;
    let schedule: String = r.get(3)?;
    let on_complete: String = r.get(4)?;
    Ok((|| {
        Ok(QueueRecord {
            id,
            settings: QueueSettings {
                name,
                max_concurrent: u32::try_from(max_concurrent)
                    .map_err(|_| StoreError::Corrupt(format!("queue limit {max_concurrent}")))?,
                schedule: serde_json::from_str(&schedule)
                    .map_err(|e| StoreError::Corrupt(format!("queue schedule: {e}")))?,
                on_complete: PostAction::from_name(&on_complete).ok_or_else(|| {
                    StoreError::Corrupt(format!("unknown queue action {on_complete:?}"))
                })?,
            },
        })
    })())
}

fn read_category(r: &Row<'_>) -> rusqlite::Result<Result<CategoryRecord>> {
    let id = CategoryId(r.get(0)?);
    let name: String = r.get(1)?;
    let rule: String = r.get(2)?;
    let save_dir: Option<String> = r.get(3)?;
    Ok(serde_json::from_str(&rule)
        .map_err(|e| StoreError::Corrupt(format!("category rule: {e}")))
        .map(|rule| CategoryRecord {
            id,
            settings: CategorySettings {
                name,
                rule,
                save_dir,
            },
        }))
}

fn schedule_json(schedule: &Schedule) -> Result<String> {
    serde_json::to_string(schedule).map_err(|e| StoreError::Corrupt(e.to_string()))
}

fn rule_json(rule: &CategoryRule) -> Result<String> {
    serde_json::to_string(rule).map_err(|e| StoreError::Corrupt(e.to_string()))
}

fn insert_category(conn: &Connection, settings: &CategorySettings) -> Result<CategoryId> {
    conn.execute(
        "INSERT INTO categories (name, position, rule_json, save_dir)
         VALUES (?1, (SELECT COALESCE(MAX(position), -1) + 1 FROM categories), ?2, ?3)",
        params![settings.name, rule_json(&settings.rule)?, settings.save_dir],
    )?;
    Ok(CategoryId(conn.last_insert_rowid()))
}

/// Adds the default categories, saving to the general downloads folder until the user
/// gives them folders of their own.
fn seed_categories(tx: &Transaction<'_>) -> Result<()> {
    for (name, rule) in omnidm_queue::category::default_categories() {
        insert_category(
            tx,
            &CategorySettings {
                name: name.into(),
                rule,
                save_dir: None,
            },
        )?;
    }
    Ok(())
}

/// Runs `update` (`?1` = id, `?2` = position) for each id, numbering them from 0.
fn renumber(tx: &Transaction<'_>, update: &str, ids: impl Iterator<Item = i64>) -> Result<()> {
    let mut stmt = tx.prepare(update)?;
    for (position, id) in ids.enumerate() {
        stmt.execute(params![id, position as i64])?;
    }
    Ok(())
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Queued => "queued",
        Status::Probing => "probing",
        Status::Running => "running",
        Status::Paused => "paused",
        Status::Completed => "completed",
        Status::Failed => "failed",
        Status::Cancelled => "cancelled",
    }
}

fn parse_status(name: &str) -> Result<Status> {
    Ok(match name {
        "queued" => Status::Queued,
        "probing" => Status::Probing,
        "running" => Status::Running,
        "paused" => Status::Paused,
        "completed" => Status::Completed,
        "failed" => Status::Failed,
        "cancelled" => Status::Cancelled,
        other => return Err(StoreError::Corrupt(format!("unknown status {other:?}"))),
    })
}

fn to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| StoreError::OutOfRange(value))
}

fn to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| StoreError::Corrupt(format!("negative size {value}")))
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
