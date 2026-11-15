//! One SQLite file for everything the tool keeps: Backend credentials, the
//! per-Backend Device cache, and the protected-Device list.
//!
//! The file is created owner-only (mode 600) and migrated by
//! `PRAGMA user_version`, so later versions can add tables without a
//! migration framework.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

/// A Backend name and a Device ID, as every table keys them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub backend: String,
    pub device_id: String,
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "opening the database: {e}"),
            StoreError::Sqlite(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

/// The newest schema this binary knows. Bump it when `migrate` gains a step.
const SCHEMA_VERSION: i64 = 1;

pub struct Store {
    conn: Connection,
    path: PathBuf,
}

impl Store {
    /// Open (creating, with mode 600) and migrate the database at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // A file created before this code ran may be wider; narrow it.
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        drop(file);
        let conn = Connection::open(path)?;
        Self::prepare(conn, path)
    }

    /// An in-memory database, for tests that do not need the file itself.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::prepare(conn, PathBuf::from(":memory:"))
    }

    fn prepare(conn: Connection, path: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let mut store = Store {
            conn,
            path: path.into(),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Apply every step from the file's `user_version` up to [`SCHEMA_VERSION`].
    fn migrate(&mut self) -> Result<(), StoreError> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StoreError::Sqlite(rusqlite::Error::InvalidQuery));
        }
        if version < 1 {
            self.conn.execute_batch(
                "BEGIN;
                 CREATE TABLE credentials (
                     backend TEXT PRIMARY KEY,
                     json    TEXT NOT NULL
                 );
                 CREATE TABLE device_cache (
                     backend    TEXT PRIMARY KEY,
                     json       TEXT NOT NULL,
                     fetched_at INTEGER NOT NULL
                 );
                 CREATE TABLE protected_devices (
                     backend      TEXT NOT NULL,
                     device_id    TEXT NOT NULL,
                     protected_at INTEGER NOT NULL,
                     PRIMARY KEY (backend, device_id)
                 );
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?;
        }
        Ok(())
    }

    /// The stored credential blob for `backend`, if any.
    pub fn credential(&self, backend: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT json FROM credentials WHERE backend = ?1",
                [backend],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_credential(&self, backend: &str, json: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO credentials (backend, json) VALUES (?1, ?2)
             ON CONFLICT(backend) DO UPDATE SET json = excluded.json",
            params![backend, json],
        )?;
        Ok(())
    }

    pub fn delete_credential(&self, backend: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM credentials WHERE backend = ?1", [backend])?;
        Ok(())
    }

    /// Remember the last Device list read from `backend`, as JSON.
    pub fn cache_devices(&self, backend: &str, json: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO device_cache (backend, json, fetched_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(backend) DO UPDATE SET json = excluded.json,
                                               fetched_at = excluded.fetched_at",
            params![backend, json, now_unix()],
        )?;
        Ok(())
    }

    pub fn cached_devices(&self, backend: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT json FROM device_cache WHERE backend = ?1",
                [backend],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn is_protected(&self, backend: &str, device_id: &str) -> Result<bool, StoreError> {
        let hit: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM protected_devices WHERE backend = ?1 AND device_id = ?2",
                params![backend, device_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(hit.is_some())
    }

    /// Add a Device to the protected list. Idempotent.
    pub fn protect(&self, backend: &str, device_id: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO protected_devices (backend, device_id, protected_at)
             VALUES (?1, ?2, ?3)",
            params![backend, device_id, now_unix()],
        )?;
        Ok(())
    }

    /// Remove a Device from the protected list; `true` when it was on it.
    pub fn unprotect(&self, backend: &str, device_id: &str) -> Result<bool, StoreError> {
        let removed = self.conn.execute(
            "DELETE FROM protected_devices WHERE backend = ?1 AND device_id = ?2",
            params![backend, device_id],
        )?;
        Ok(removed > 0)
    }

    /// Every protected Device, stable order.
    pub fn protected_devices(&self) -> Result<Vec<Pair>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT backend, device_id FROM protected_devices ORDER BY backend, device_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Pair {
                backend: row.get(0)?,
                device_id: row.get(1)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_is_owner_only_and_at_the_newest_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home.sqlite");
        let store = Store::open(&path).unwrap();
        let version: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn reopening_keeps_the_data_and_does_not_remigrate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home.sqlite");
        {
            let store = Store::open(&path).unwrap();
            store
                .set_credential("google_home", r#"{"master":"dummy"}"#)
                .unwrap();
            store.protect("google_home", "dev-1").unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.credential("google_home").unwrap().as_deref(),
            Some(r#"{"master":"dummy"}"#)
        );
        assert!(store.is_protected("google_home", "dev-1").unwrap());
    }

    #[test]
    fn protection_is_per_backend_and_idempotent() {
        let store = Store::open_in_memory().unwrap();
        store.protect("google_home", "dev-1").unwrap();
        store.protect("google_home", "dev-1").unwrap();
        assert!(store.is_protected("google_home", "dev-1").unwrap());
        assert!(!store.is_protected("tuya", "dev-1").unwrap());
        assert_eq!(store.protected_devices().unwrap().len(), 1);
        assert!(store.unprotect("google_home", "dev-1").unwrap());
        assert!(!store.unprotect("google_home", "dev-1").unwrap());
        assert!(store.protected_devices().unwrap().is_empty());
    }

    #[test]
    fn the_device_cache_round_trips() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.cached_devices("google_home").unwrap().is_none());
        store
            .cache_devices("google_home", r#"[{"id":"a"}]"#)
            .unwrap();
        store
            .cache_devices("google_home", r#"[{"id":"b"}]"#)
            .unwrap();
        assert_eq!(
            store.cached_devices("google_home").unwrap().as_deref(),
            Some(r#"[{"id":"b"}]"#)
        );
    }
}
