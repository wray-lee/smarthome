//! One SQLite file for everything the tool keeps: Backend credentials, the
//! per-Backend Device cache, the protected-Device list, and the Tuya Local
//! Keys with the history of replaced ones.
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

/// A Tuya Device's current Local Key and when it was first seen and written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalKeyRecord {
    pub device_id: String,
    pub local_key: String,
    /// When this Device's key was saved for the first time.
    pub first_seen_at: i64,
    /// The last time this row was written: first save, a later observation of
    /// the same key, or a key change.
    pub changed_at: i64,
}

/// One replaced Local Key, kept so a re-paired Device does not lose the old
/// key and the time of the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalKeyChange {
    pub old_key: String,
    pub changed_at: i64,
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
const SCHEMA_VERSION: i64 = 2;

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
        if version < 2 {
            self.conn.execute_batch(
                "BEGIN;
                 CREATE TABLE tuya_local_keys (
                     device_id     TEXT PRIMARY KEY,
                     local_key     TEXT NOT NULL,
                     first_seen_at INTEGER NOT NULL,
                     changed_at    INTEGER NOT NULL
                 );
                 CREATE TABLE tuya_local_key_history (
                     id         INTEGER PRIMARY KEY AUTOINCREMENT,
                     device_id  TEXT NOT NULL,
                     old_key    TEXT NOT NULL,
                     changed_at INTEGER NOT NULL
                 );
                 PRAGMA user_version = 2;
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

    /// Record the Local Key seen for a Tuya Device at `now_unix`.
    ///
    /// The first save stores the key with `first_seen_at` and `changed_at`
    /// set to `now_unix`. Seeing the same key again only refreshes the
    /// observation time and records nothing. A different key replaces the
    /// stored one, keeps `first_seen_at`, records the old key and the time it
    /// was replaced in `tuya_local_key_history`, and sets `changed_at` to
    /// `now_unix`.
    ///
    /// Returns `true` when the effective key changed (a new Device or a
    /// different key), `false` when it was another sighting of the same key.
    pub fn save_local_key(
        &self,
        device_id: &str,
        local_key: &str,
        now_unix: i64,
    ) -> Result<bool, StoreError> {
        let previous: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT local_key, first_seen_at FROM tuya_local_keys WHERE device_id = ?1",
                [device_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match previous {
            None => {
                self.conn.execute(
                    "INSERT INTO tuya_local_keys
                         (device_id, local_key, first_seen_at, changed_at)
                     VALUES (?1, ?2, ?3, ?3)",
                    params![device_id, local_key, now_unix],
                )?;
                Ok(true)
            }
            Some((old_key, _first_seen_at)) if old_key == local_key => {
                self.conn.execute(
                    "UPDATE tuya_local_keys SET changed_at = ?2 WHERE device_id = ?1",
                    params![device_id, now_unix],
                )?;
                Ok(false)
            }
            Some((old_key, _first_seen_at)) => {
                // One transaction, so a crash cannot keep the old key out of
                // the history while the new one is already current.
                let tx = self.conn.unchecked_transaction()?;
                tx.execute(
                    "INSERT INTO tuya_local_key_history (device_id, old_key, changed_at)
                     VALUES (?1, ?2, ?3)",
                    params![device_id, old_key, now_unix],
                )?;
                tx.execute(
                    "UPDATE tuya_local_keys
                     SET local_key = ?2, changed_at = ?3
                     WHERE device_id = ?1",
                    params![device_id, local_key, now_unix],
                )?;
                tx.commit()?;
                Ok(true)
            }
        }
    }

    /// Every saved Local Key, ordered by Device ID so the export is stable.
    pub fn local_keys(&self) -> Result<Vec<LocalKeyRecord>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT device_id, local_key, first_seen_at, changed_at
             FROM tuya_local_keys ORDER BY device_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(LocalKeyRecord {
                device_id: row.get(0)?,
                local_key: row.get(1)?,
                first_seen_at: row.get(2)?,
                changed_at: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The keys `device_id` used to have, oldest change first. For tests and
    /// for anyone auditing a rotated Device.
    pub fn local_key_history(&self, device_id: &str) -> Result<Vec<LocalKeyChange>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT old_key, changed_at FROM tuya_local_key_history
             WHERE device_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([device_id], |row| {
            Ok(LocalKeyChange {
                old_key: row.get(0)?,
                changed_at: row.get(1)?,
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

    /// A database as the v1 code left it: schema version 1, no Local Key
    /// tables, one row in every old table.
    fn write_v1_file(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE credentials (
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
             INSERT INTO credentials VALUES ('google_home', '{\"master\":\"dummy\"}');
             INSERT INTO device_cache VALUES ('google_home', '[{\"id\":\"dev-1\"}]', 100);
             INSERT INTO protected_devices VALUES ('google_home', 'dev-1', 50);",
        )
        .unwrap();
    }

    #[test]
    fn a_v1_file_migrates_to_v2_keeping_every_old_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home.sqlite");
        write_v1_file(&path);
        let store = Store::open(&path).unwrap();
        let version: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
        assert_eq!(
            store.credential("google_home").unwrap().as_deref(),
            Some(r#"{"master":"dummy"}"#)
        );
        assert_eq!(
            store.cached_devices("google_home").unwrap().as_deref(),
            Some(r#"[{"id":"dev-1"}]"#)
        );
        assert!(store.is_protected("google_home", "dev-1").unwrap());
        // The new tables exist and start empty.
        assert!(store.local_keys().unwrap().is_empty());
        assert!(store.local_key_history("dev-1").unwrap().is_empty());
        assert!(store.save_local_key("dev-1", "key-a", 10).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn a_new_file_is_born_at_v2_with_the_local_key_tables() {
        let store = Store::open_in_memory().unwrap();
        let version: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
        assert!(store.local_keys().unwrap().is_empty());
    }

    #[test]
    fn the_first_save_records_the_key_and_both_times() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.save_local_key("dev-1", "key-a", 100).unwrap());
        assert_eq!(
            store.local_keys().unwrap(),
            [LocalKeyRecord {
                device_id: "dev-1".into(),
                local_key: "key-a".into(),
                first_seen_at: 100,
                changed_at: 100,
            }]
        );
        assert!(store.local_key_history("dev-1").unwrap().is_empty());
    }

    #[test]
    fn seeing_the_same_key_again_writes_no_history() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.save_local_key("dev-1", "key-a", 100).unwrap());
        assert!(!store.save_local_key("dev-1", "key-a", 200).unwrap());
        let keys = store.local_keys().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].local_key, "key-a");
        assert_eq!(keys[0].first_seen_at, 100);
        assert_eq!(keys[0].changed_at, 200);
        assert!(store.local_key_history("dev-1").unwrap().is_empty());
    }

    #[test]
    fn a_changed_key_keeps_the_old_value_and_the_time_it_changed() {
        let store = Store::open_in_memory().unwrap();
        store.save_local_key("dev-1", "key-a", 100).unwrap();
        assert!(store.save_local_key("dev-1", "key-b", 300).unwrap());
        assert_eq!(
            store.local_keys().unwrap(),
            [LocalKeyRecord {
                device_id: "dev-1".into(),
                local_key: "key-b".into(),
                first_seen_at: 100,
                changed_at: 300,
            }]
        );
        assert_eq!(
            store.local_key_history("dev-1").unwrap(),
            [LocalKeyChange {
                old_key: "key-a".into(),
                changed_at: 300,
            }]
        );
        // A second change appends rather than overwrites.
        assert!(store.save_local_key("dev-1", "key-c", 400).unwrap());
        let history = store.local_key_history("dev-1").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].old_key, "key-b");
        assert_eq!(history[1].changed_at, 400);
    }

    #[test]
    fn local_keys_export_in_stable_device_id_order() {
        let store = Store::open_in_memory().unwrap();
        store.save_local_key("dev-b", "key-b", 10).unwrap();
        store.save_local_key("dev-a", "key-a", 20).unwrap();
        store.save_local_key("dev-c", "key-c", 30).unwrap();
        let ids: Vec<String> = store
            .local_keys()
            .unwrap()
            .into_iter()
            .map(|r| r.device_id)
            .collect();
        assert_eq!(ids, ["dev-a", "dev-b", "dev-c"]);
    }

    #[test]
    fn local_keys_survive_reopening_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home.sqlite");
        {
            let store = Store::open(&path).unwrap();
            store.save_local_key("dev-1", "key-a", 100).unwrap();
            store.save_local_key("dev-1", "key-b", 200).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.local_keys().unwrap()[0].local_key, "key-b");
        assert_eq!(
            store.local_key_history("dev-1").unwrap()[0].old_key,
            "key-a"
        );
    }
}
