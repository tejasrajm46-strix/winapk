//! Persistence. The runtime is authoritative for what is installed; this store
//! only remembers preferences, labels/icons we already resolved, and history.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::domain::{DwError, DwResult, ErrorCode};

const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedApp {
    pub package_id: String,
    pub label: Option<String>,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
    pub icon: Option<String>,
    /// Manifest target SDK, so an old package can be labelled after a restart.
    pub target_sdk: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRecord {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub summary: String,
    pub package_id: Option<String>,
    pub detail: Option<String>,
}

impl OperationRecord {
    /// A record for an operation that has already ended.
    pub fn finished(
        op: &str,
        kind: &str,
        status: &str,
        started_at: i64,
        summary: impl Into<String>,
        package_id: Option<String>,
        detail: Option<String>,
    ) -> Self {
        Self {
            id: op.to_string(),
            kind: kind.to_string(),
            status: status.to_string(),
            started_at,
            finished_at: Some(crate::domain::now_ms()),
            summary: summary.into(),
            package_id,
            detail,
        }
    }
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> DwResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DwError::io(
                    ErrorCode::StorageFailure,
                    "Cannot create the data directory.",
                    &e,
                )
            })?;
        }
        let conn = Connection::open(path).map_err(|e| {
            DwError::new(ErrorCode::StorageFailure, "Cannot open the local database.")
                .detail(e.to_string())
        })?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> DwResult<Self> {
        let conn = Connection::open_in_memory().map_err(|e| {
            DwError::new(ErrorCode::StorageFailure, "Cannot open the local database.")
                .detail(e.to_string())
        })?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    fn conn(&self) -> DwResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| {
            DwError::new(
                ErrorCode::StorageFailure,
                "The local database lock was poisoned.",
            )
        })
    }

    fn migrate(&self) -> DwResult<()> {
        let conn = self.conn()?;
        let current: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(storage_err)?;
        if current >= SCHEMA_VERSION {
            return Ok(());
        }
        if (1..2).contains(&current) {
            // v1 databases predate the target-SDK column; keep their rows.
            conn.execute_batch(
                "BEGIN;
                 ALTER TABLE app_cache ADD COLUMN target_sdk INTEGER;
                 PRAGMA user_version = 2;
                 COMMIT;",
            )
            .map_err(storage_err)?;
            return Ok(());
        }
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS settings (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS app_cache (
                 package_id   TEXT PRIMARY KEY,
                 label        TEXT,
                 version_name TEXT,
                 version_code INTEGER,
                 icon         TEXT,
                 target_sdk   INTEGER,
                 last_seen_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS operation_history (
                 id          TEXT PRIMARY KEY,
                 kind        TEXT NOT NULL,
                 status      TEXT NOT NULL,
                 started_at  INTEGER NOT NULL,
                 finished_at INTEGER,
                 summary     TEXT NOT NULL,
                 package_id  TEXT,
                 detail      TEXT
             );
             CREATE TABLE IF NOT EXISTS launch_history (
                 package_id  TEXT NOT NULL,
                 launched_at INTEGER NOT NULL
             );
             PRAGMA user_version = 2;
             COMMIT;",
        )
        .map_err(storage_err)?;
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        let conn = self.conn().ok()?;
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> DwResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(storage_err)?;
        Ok(())
    }

    pub fn cached_apps(&self) -> Vec<CachedApp> {
        let Ok(conn) = self.conn() else {
            return Vec::new();
        };
        let mut stmt = match conn.prepare(
            "SELECT package_id, label, version_name, version_code, icon, target_sdk FROM app_cache",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |row| {
            Ok(CachedApp {
                package_id: row.get(0)?,
                label: row.get(1)?,
                version_name: row.get(2)?,
                version_code: row.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                icon: row.get(4)?,
                target_sdk: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
            })
        });
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn cache_app(&self, app: &CachedApp, seen_at: i64) -> DwResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO app_cache
                (package_id, label, version_name, version_code, icon, target_sdk, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(package_id) DO UPDATE SET
                label = COALESCE(excluded.label, app_cache.label),
                version_name = excluded.version_name,
                version_code = excluded.version_code,
                icon = COALESCE(excluded.icon, app_cache.icon),
                target_sdk = COALESCE(excluded.target_sdk, app_cache.target_sdk),
                last_seen_at = excluded.last_seen_at",
            params![
                app.package_id,
                app.label,
                app.version_name,
                app.version_code.map(|v| v as i64),
                app.icon,
                app.target_sdk.map(|v| v as i64),
                seen_at
            ],
        )
        .map_err(storage_err)?;
        Ok(())
    }

    pub fn forget_app(&self, package_id: &str) -> DwResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM app_cache WHERE package_id = ?1",
            params![package_id],
        )
        .map_err(storage_err)?;
        Ok(())
    }

    pub fn record_operation(&self, record: &OperationRecord) -> DwResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO operation_history
                (id, kind, status, started_at, finished_at, summary, package_id, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                finished_at = excluded.finished_at,
                summary = excluded.summary,
                detail = excluded.detail",
            params![
                record.id,
                record.kind,
                record.status,
                record.started_at,
                record.finished_at,
                record.summary,
                record.package_id,
                record.detail
            ],
        )
        .map_err(storage_err)?;
        Ok(())
    }

    pub fn operations(&self, limit: usize) -> Vec<OperationRecord> {
        let Ok(conn) = self.conn() else {
            return Vec::new();
        };
        let mut stmt = match conn.prepare(
            "SELECT id, kind, status, started_at, finished_at, summary, package_id, detail
             FROM operation_history ORDER BY started_at DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(OperationRecord {
                id: row.get(0)?,
                kind: row.get(1)?,
                status: row.get(2)?,
                started_at: row.get(3)?,
                finished_at: row.get(4)?,
                summary: row.get(5)?,
                package_id: row.get(6)?,
                detail: row.get(7)?,
            })
        });
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn record_launch(&self, package_id: &str, at: i64) -> DwResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO launch_history (package_id, launched_at) VALUES (?1, ?2)",
            params![package_id, at],
        )
        .map_err(storage_err)?;
        Ok(())
    }
}

fn storage_err(e: rusqlite::Error) -> DwError {
    DwError::new(
        ErrorCode::StorageFailure,
        "The local database rejected that change.",
    )
    .detail(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(package_id: &str) -> CachedApp {
        CachedApp {
            package_id: package_id.into(),
            label: Some("Demo".into()),
            version_name: Some("1.0".into()),
            version_code: Some(1),
            icon: None,
            target_sdk: Some(36),
        }
    }

    #[test]
    fn migrations_are_idempotent() {
        let dir = std::env::temp_dir().join(format!("droidwin-store-{}", std::process::id()));
        let path = dir.join("test.db");
        let _ = std::fs::remove_dir_all(&dir);
        Store::open(&path).expect("first open");
        let store = Store::open(&path).expect("second open");
        let _ = store.set_setting("k", "v");
        assert_eq!(store.setting("k").as_deref(), Some("v"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn labels_survive_reinstall_without_icon() {
        let store = Store::open_in_memory().unwrap();
        store.cache_app(&sample("com.a"), 1).unwrap();
        let mut updated = sample("com.a");
        updated.label = None; // runtime could not resolve the label this time
        updated.version_code = Some(2);
        store.cache_app(&updated, 2).unwrap();
        let cached = store.cached_apps();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].label.as_deref(), Some("Demo"));
        assert_eq!(cached[0].version_code, Some(2));
    }

    #[test]
    fn a_v1_database_gains_the_target_sdk_column_without_losing_rows() {
        let dir = std::env::temp_dir().join(format!("droidwin-migrate-{}", std::process::id()));
        let path = dir.join("v1.db");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE app_cache (
                     package_id TEXT PRIMARY KEY, label TEXT, version_name TEXT,
                     version_code INTEGER, icon TEXT, last_seen_at INTEGER NOT NULL
                 );
                 INSERT INTO app_cache VALUES ('com.old', 'Old', '1.0', 1, NULL, 1);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let cached = store.cached_apps();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].label.as_deref(), Some("Old"));
        assert_eq!(cached[0].target_sdk, None);
        let version: i64 = store
            .conn()
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn target_sdk_survives_a_reinstall_without_a_manifest_value() {
        let store = Store::open_in_memory().unwrap();
        store.cache_app(&sample("com.a"), 1).unwrap();
        let mut updated = sample("com.a");
        updated.target_sdk = None;
        store.cache_app(&updated, 2).unwrap();
        assert_eq!(store.cached_apps()[0].target_sdk, Some(36));
    }

    #[test]
    fn operations_are_ordered_newest_first() {
        let store = Store::open_in_memory().unwrap();
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            store
                .record_operation(&OperationRecord {
                    id: (*id).into(),
                    kind: "install".into(),
                    status: "completed".into(),
                    started_at: i as i64,
                    finished_at: Some(i as i64),
                    summary: "ok".into(),
                    package_id: None,
                    detail: None,
                })
                .unwrap();
        }
        let ops = store.operations(2);
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].id, "c");
        assert_eq!(ops[1].id, "b");
    }
}
