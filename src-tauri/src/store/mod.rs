//! Persistence. The runtime is authoritative for what is installed; this store
//! only remembers preferences, labels/icons we already resolved, and history.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::domain::{AppOrientation, DwError, DwResult, ErrorCode};

const SCHEMA_VERSION: i64 = 5;

/// A trigger as stored: its condition and its steps are JSON so the row stays a
/// single row, exactly like `input_mappings` keeps an action in two columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTrigger {
    pub id: String,
    pub name: String,
    /// The package this trigger watches; None means the runtime itself.
    pub package_id: Option<String>,
    /// `manual`, `app_opened`, `app_closed`, `app_crashed`, `runtime_failed`,
    /// `element_present`, `element_absent` or `app_foreground`.
    pub when_kind: String,
    pub condition: String,
    pub steps: String,
    pub enabled: bool,
    pub fire_count: u64,
    pub last_fired_at: Option<i64>,
    pub created_at: i64,
}

/// Per-app key mapping. `key` is a Windows virtual-key code, which is what the
/// low-level keyboard hook reports; `action`/`target` keep the action in a form
/// one row can hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredMapping {
    pub package_id: String,
    pub key: u32,
    pub action: String,
    pub target: String,
    pub enabled: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedApp {
    pub package_id: String,
    pub label: Option<String>,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
    pub icon: Option<String>,
    /// Manifest target SDK, so an old package can be labelled after a restart.
    pub target_sdk: Option<u32>,
    /// Orientation this package is drawn in. From its manifest when WINAPK
    /// installed it; recorded from Android's own display when it did not.
    pub orientation: Option<AppOrientation>,
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
        if current == 0 {
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
                     orientation  TEXT,
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
                 CREATE TABLE IF NOT EXISTS input_mappings (
                     package_id TEXT NOT NULL,
                     key        INTEGER NOT NULL,
                     action     TEXT NOT NULL,
                     target     TEXT NOT NULL,
                     enabled    INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL,
                     PRIMARY KEY (package_id, key)
                 );
                 CREATE TABLE IF NOT EXISTS triggers (
                     id            TEXT PRIMARY KEY,
                     name          TEXT NOT NULL,
                     package_id    TEXT,
                     when_kind     TEXT NOT NULL,
                     condition     TEXT NOT NULL DEFAULT '{}',
                     steps         TEXT NOT NULL,
                     enabled       INTEGER NOT NULL,
                     fire_count    INTEGER NOT NULL DEFAULT 0,
                     last_fired_at INTEGER,
                     created_at    INTEGER NOT NULL
                 );
                 PRAGMA user_version = 5;
                 COMMIT;",
            )
            .map_err(storage_err)?;
            return Ok(());
        }
        // Upgrades add columns; existing rows are kept, not rebuilt.
        let mut batch = String::from("BEGIN;");
        if current < 2 {
            batch.push_str("ALTER TABLE app_cache ADD COLUMN target_sdk INTEGER;");
        }
        if current < 3 {
            batch.push_str("ALTER TABLE app_cache ADD COLUMN orientation TEXT;");
        }
        if current < 4 {
            batch.push_str(
                "CREATE TABLE IF NOT EXISTS input_mappings (
                     package_id TEXT NOT NULL,
                     key        INTEGER NOT NULL,
                     action     TEXT NOT NULL,
                     target     TEXT NOT NULL,
                     enabled    INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL,
                     PRIMARY KEY (package_id, key)
                 );",
            );
        }
        if current < 5 {
            batch.push_str(
                "CREATE TABLE IF NOT EXISTS triggers (
                     id            TEXT PRIMARY KEY,
                     name          TEXT NOT NULL,
                     package_id    TEXT,
                     when_kind     TEXT NOT NULL,
                     condition     TEXT NOT NULL DEFAULT '{}',
                     steps         TEXT NOT NULL,
                     enabled       INTEGER NOT NULL,
                     fire_count    INTEGER NOT NULL DEFAULT 0,
                     last_fired_at INTEGER,
                     created_at    INTEGER NOT NULL
                 );",
            );
        }
        batch.push_str(&format!("PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"));
        conn.execute_batch(&batch).map_err(storage_err)?;
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
            "SELECT package_id, label, version_name, version_code, icon, target_sdk, orientation
             FROM app_cache",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], cached_app_row);
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// One cached row. A launch reads this instead of scanning the whole cache.
    pub fn cached_app(&self, package_id: &str) -> Option<CachedApp> {
        let conn = self.conn().ok()?;
        conn.query_row(
            "SELECT package_id, label, version_name, version_code, icon, target_sdk, orientation
             FROM app_cache WHERE package_id = ?1",
            params![package_id],
            cached_app_row,
        )
        .optional()
        .ok()
        .flatten()
    }

    pub fn cache_app(&self, app: &CachedApp, seen_at: i64) -> DwResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO app_cache
                (package_id, label, version_name, version_code, icon, target_sdk, orientation, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(package_id) DO UPDATE SET
                label = COALESCE(excluded.label, app_cache.label),
                version_name = excluded.version_name,
                version_code = excluded.version_code,
                icon = COALESCE(excluded.icon, app_cache.icon),
                target_sdk = COALESCE(excluded.target_sdk, app_cache.target_sdk),
                orientation = COALESCE(excluded.orientation, app_cache.orientation),
                last_seen_at = excluded.last_seen_at",
            params![
                app.package_id,
                app.label,
                app.version_name,
                app.version_code.map(|v| v as i64),
                app.icon,
                app.target_sdk.map(|v| v as i64),
                app.orientation.map(|o| o.as_str()),
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

    /// One package's mappings, ordered by key so the editor does not reshuffle.
    pub fn input_mappings(&self, package_id: &str) -> Vec<StoredMapping> {
        let Ok(conn) = self.conn() else {
            return Vec::new();
        };
        let mut stmt = match conn.prepare(
            "SELECT package_id, key, action, target, enabled, updated_at
             FROM input_mappings WHERE package_id = ?1 ORDER BY key",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map(params![package_id], |row| {
            Ok(StoredMapping {
                package_id: row.get(0)?,
                key: row.get::<_, i64>(1)? as u32,
                action: row.get(2)?,
                target: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                updated_at: row.get(5)?,
            })
        });
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Insert or replace one key's mapping. A key maps to one action per app.
    pub fn save_input_mapping(&self, mapping: &StoredMapping) -> DwResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO input_mappings (package_id, key, action, target, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(package_id, key) DO UPDATE SET
                action = excluded.action,
                target = excluded.target,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at",
            params![
                mapping.package_id,
                mapping.key as i64,
                mapping.action,
                mapping.target,
                if mapping.enabled { 1 } else { 0 },
                mapping.updated_at
            ],
        )
        .map_err(storage_err)?;
        Ok(())
    }

    pub fn remove_input_mapping(&self, package_id: &str, key: u32) -> DwResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM input_mappings WHERE package_id = ?1 AND key = ?2",
            params![package_id, key as i64],
        )
        .map_err(storage_err)?;
        Ok(())
    }

    pub fn clear_input_mappings(&self, package_id: &str) -> DwResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM input_mappings WHERE package_id = ?1",
            params![package_id],
        )
        .map_err(storage_err)?;
        Ok(())
    }

    /// Mapping is off unless the user turned it on for this app.
    pub fn input_mapping_enabled(&self, package_id: &str) -> bool {
        self.setting(&input_enabled_key(package_id)).as_deref() == Some("1")
    }

    pub fn set_input_mapping_enabled(&self, package_id: &str, enabled: bool) -> DwResult<()> {
        self.set_setting(
            &input_enabled_key(package_id),
            if enabled { "1" } else { "0" },
        )
    }

    /// Packages whose mapping is armed, read from the store rather than memory so
    /// a restart cannot leave a profile silently active.
    pub fn input_enabled_packages(&self) -> Vec<String> {
        let Ok(conn) = self.conn() else {
            return Vec::new();
        };
        let mut stmt = match conn
            .prepare("SELECT key FROM settings WHERE key LIKE ?1 AND value = '1' ORDER BY key")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map(params![format!("{INPUT_ENABLED_PREFIX}%")], |row| {
            row.get::<_, String>(0)
        });
        match rows {
            Ok(iter) => iter
                .filter_map(|r| r.ok())
                .map(|key| key.trim_start_matches(INPUT_ENABLED_PREFIX).to_string())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Insert or replace one trigger. Its id is the identity a client keeps across
/// restarts, so an edit keeps the same row and therefore the same fire history.
impl Store {
    pub fn save_trigger(&self, trigger: &StoredTrigger) -> DwResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO triggers
                 (id, name, package_id, when_kind, condition, steps, enabled, fire_count, last_fired_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 package_id = excluded.package_id,
                 when_kind = excluded.when_kind,
                 condition = excluded.condition,
                 steps = excluded.steps,
                 enabled = excluded.enabled,
                 fire_count = excluded.fire_count,
                 last_fired_at = excluded.last_fired_at",
            params![
                trigger.id,
                trigger.name,
                trigger.package_id,
                trigger.when_kind,
                trigger.condition,
                trigger.steps,
                i64::from(trigger.enabled),
                trigger.fire_count as i64,
                trigger.last_fired_at,
                trigger.created_at,
            ],
        )
        .map_err(storage_err)?;
        Ok(())
    }

    pub fn triggers(&self) -> Vec<StoredTrigger> {
        let Ok(conn) = self.conn() else {
            return Vec::new();
        };
        let mut stmt = match conn.prepare(
            "SELECT id, name, package_id, when_kind, condition, steps, enabled,
                    fire_count, last_fired_at, created_at
             FROM triggers ORDER BY created_at, id",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], trigger_row);
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn trigger(&self, id: &str) -> Option<StoredTrigger> {
        let conn = self.conn().ok()?;
        conn.query_row(
            "SELECT id, name, package_id, when_kind, condition, steps, enabled,
                    fire_count, last_fired_at, created_at
             FROM triggers WHERE id = ?1",
            params![id],
            trigger_row,
        )
        .optional()
        .ok()
        .flatten()
    }

    pub fn remove_trigger(&self, id: &str) -> DwResult<bool> {
        let conn = self.conn()?;
        let removed = conn
            .execute("DELETE FROM triggers WHERE id = ?1", params![id])
            .map_err(storage_err)?;
        Ok(removed > 0)
    }

    /// The fire itself is history: a trigger the user armed should show that it
    /// ran and when, even if its action then failed.
    pub fn record_trigger_fired(&self, id: &str, at: i64) -> DwResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE triggers SET fire_count = fire_count + 1, last_fired_at = ?2 WHERE id = ?1",
            params![id, at],
        )
        .map_err(storage_err)?;
        Ok(())
    }
}

const INPUT_ENABLED_PREFIX: &str = "input.enabled:";

fn trigger_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredTrigger> {
    Ok(StoredTrigger {
        id: row.get(0)?,
        name: row.get(1)?,
        package_id: row.get(2)?,
        when_kind: row.get(3)?,
        condition: row.get(4)?,
        steps: row.get(5)?,
        enabled: row.get::<_, i64>(6)? != 0,
        fire_count: row.get::<_, i64>(7)? as u64,
        last_fired_at: row.get(8)?,
        created_at: row.get(9)?,
    })
}

fn input_enabled_key(package_id: &str) -> String {
    format!("{INPUT_ENABLED_PREFIX}{package_id}")
}

fn cached_app_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CachedApp> {
    Ok(CachedApp {
        package_id: row.get(0)?,
        label: row.get(1)?,
        version_name: row.get(2)?,
        version_code: row.get::<_, Option<i64>>(3)?.map(|v| v as u64),
        icon: row.get(4)?,
        target_sdk: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
        orientation: row
            .get::<_, Option<String>>(6)?
            .and_then(|v| AppOrientation::parse(&v)),
    })
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
            orientation: None,
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
        assert_eq!(version, SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_v2_database_gains_the_orientation_column_without_losing_rows() {
        let dir = std::env::temp_dir().join(format!("droidwin-migrate-v2-{}", std::process::id()));
        let path = dir.join("v2.db");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE app_cache (
                     package_id TEXT PRIMARY KEY, label TEXT, version_name TEXT,
                     version_code INTEGER, icon TEXT, target_sdk INTEGER,
                     last_seen_at INTEGER NOT NULL
                 );
                 INSERT INTO app_cache VALUES ('org.example.game', 'Game', '1.0', 3, NULL, 35, 1);
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let cached = store.cached_apps();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].label.as_deref(), Some("Game"));
        assert_eq!(cached[0].target_sdk, Some(35));
        assert_eq!(cached[0].orientation, None);

        // The new column is writable on an upgraded database.
        store
            .cache_app(
                &CachedApp {
                    orientation: Some(AppOrientation::Landscape),
                    ..cached[0].clone()
                },
                2,
            )
            .unwrap();
        assert_eq!(
            store.cached_app("org.example.game").unwrap().orientation,
            Some(AppOrientation::Landscape)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_orientation_survives_a_reinstall_that_could_not_read_one() {
        let store = Store::open_in_memory().unwrap();
        let mut app = sample("org.example.game");
        app.orientation = Some(AppOrientation::Landscape);
        store.cache_app(&app, 1).unwrap();
        // A later install of the same package whose manifest could not be read
        // must not erase what Android already showed about its orientation.
        store.cache_app(&sample("org.example.game"), 2).unwrap();
        assert_eq!(
            store.cached_app("org.example.game").unwrap().orientation,
            Some(AppOrientation::Landscape)
        );
    }

    #[test]
    fn a_missing_package_has_no_cached_row() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.cached_app("org.example.absent").is_none());
    }

    fn mapping(package_id: &str, key: u32, action: &str, target: &str) -> StoredMapping {
        StoredMapping {
            package_id: package_id.into(),
            key,
            action: action.into(),
            target: target.into(),
            enabled: true,
            updated_at: 1,
        }
    }

    #[test]
    fn a_v3_database_gains_the_mapping_table_and_keeps_its_rows() {
        let dir = std::env::temp_dir().join(format!("droidwin-migrate-v3-{}", std::process::id()));
        let path = dir.join("v3.db");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE app_cache (
                     package_id TEXT PRIMARY KEY, label TEXT, version_name TEXT,
                     version_code INTEGER, icon TEXT, target_sdk INTEGER,
                     orientation TEXT, last_seen_at INTEGER NOT NULL
                 );
                 INSERT INTO app_cache VALUES ('org.example.game', 'Game', '1.0', 3, NULL, 35, 'landscape', 1);
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .cached_app("org.example.game")
                .unwrap()
                .label
                .as_deref(),
            Some("Game")
        );
        // The new table is usable on the upgraded database.
        store
            .save_input_mapping(&mapping("org.example.game", 74, "touch_tap", "50.0,80.0"))
            .unwrap();
        assert_eq!(store.input_mappings("org.example.game").len(), 1);
        let version: i64 = store
            .conn()
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_key_replaces_its_previous_action() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_input_mapping(&mapping("org.example.game", 74, "touch_tap", "50.0,80.0"))
            .unwrap();
        store
            .save_input_mapping(&mapping("org.example.game", 74, "key_event", "KEYCODE_A"))
            .unwrap();
        let saved = store.input_mappings("org.example.game");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].action, "key_event");
        assert_eq!(saved[0].target, "KEYCODE_A");
    }

    #[test]
    fn mappings_are_per_package_and_can_be_cleared() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_input_mapping(&mapping("org.example.a", 32, "key_event", "KEYCODE_SPACE"))
            .unwrap();
        store
            .save_input_mapping(&mapping("org.example.b", 32, "key_event", "KEYCODE_ENTER"))
            .unwrap();
        assert_eq!(
            store.input_mappings("org.example.a")[0].target,
            "KEYCODE_SPACE"
        );
        assert_eq!(
            store.input_mappings("org.example.b")[0].target,
            "KEYCODE_ENTER"
        );

        store.remove_input_mapping("org.example.a", 32).unwrap();
        assert!(store.input_mappings("org.example.a").is_empty());
        assert_eq!(store.input_mappings("org.example.b").len(), 1);

        store.clear_input_mappings("org.example.b").unwrap();
        assert!(store.input_mappings("org.example.b").is_empty());
    }

    #[test]
    fn mapping_is_off_until_it_is_turned_on_for_that_app() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_input_mapping(&mapping("org.example.game", 74, "key_event", "KEYCODE_A"))
            .unwrap();
        assert!(!store.input_mapping_enabled("org.example.game"));
        assert!(store.input_enabled_packages().is_empty());

        store
            .set_input_mapping_enabled("org.example.game", true)
            .unwrap();
        assert!(store.input_mapping_enabled("org.example.game"));
        assert_eq!(
            store.input_enabled_packages(),
            vec!["org.example.game".to_string()]
        );

        store
            .set_input_mapping_enabled("org.example.game", false)
            .unwrap();
        assert!(!store.input_mapping_enabled("org.example.game"));
        assert!(store.input_enabled_packages().is_empty());
    }

    fn trigger(id: &str) -> StoredTrigger {
        StoredTrigger {
            id: id.into(),
            name: "start the game".into(),
            package_id: Some("org.example.game".into()),
            when_kind: "app_opened".into(),
            condition: "{}".into(),
            steps: r#"[{"tool":"tap","arguments":{"x":1,"y":2}}]"#.into(),
            enabled: true,
            fire_count: 0,
            last_fired_at: None,
            created_at: 5,
        }
    }

    #[test]
    fn a_v4_database_gains_the_trigger_table_and_keeps_its_rows() {
        let dir = std::env::temp_dir().join(format!("droidwin-migrate-v4-{}", std::process::id()));
        let path = dir.join("v4.db");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE app_cache (
                     package_id TEXT PRIMARY KEY, label TEXT, version_name TEXT,
                     version_code INTEGER, icon TEXT, target_sdk INTEGER,
                     orientation TEXT, last_seen_at INTEGER NOT NULL
                 );
                 CREATE TABLE input_mappings (
                     package_id TEXT NOT NULL, key INTEGER NOT NULL,
                     action TEXT NOT NULL, target TEXT NOT NULL,
                     enabled INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                     PRIMARY KEY (package_id, key)
                 );
                 INSERT INTO app_cache VALUES ('org.example.game', 'Game', '1.0', 3, NULL, 35, 'landscape', 1);
                 INSERT INTO input_mappings VALUES ('org.example.game', 74, 'touch_tap', '50.00,80.00', 1, 2);
                 PRAGMA user_version = 4;",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .cached_app("org.example.game")
                .unwrap()
                .label
                .as_deref(),
            Some("Game")
        );
        assert_eq!(store.input_mappings("org.example.game").len(), 1);
        // The new table is usable on the upgraded database.
        store.save_trigger(&trigger("trg-1")).unwrap();
        assert_eq!(store.triggers().len(), 1);
        let version: i64 = store
            .conn()
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_trigger_round_trips_and_keeps_its_fire_history_across_an_edit() {
        let store = Store::open_in_memory().unwrap();
        store.save_trigger(&trigger("trg-1")).unwrap();
        let saved = store.trigger("trg-1").unwrap();
        assert_eq!(saved.name, "start the game");
        assert_eq!(saved.when_kind, "app_opened");
        assert!(saved.steps.contains("tap"));
        assert_eq!(saved.fire_count, 0);

        store.record_trigger_fired("trg-1", 99).unwrap();
        let fired = store.trigger("trg-1").unwrap();
        assert_eq!(fired.fire_count, 1);
        assert_eq!(fired.last_fired_at, Some(99));

        // An edit writes the row it was given; the engine is what preserves the
        // history, so the store must not invent one either.
        let mut edited = fired.clone();
        edited.name = "start it again".into();
        store.save_trigger(&edited).unwrap();
        let after = store.trigger("trg-1").unwrap();
        assert_eq!(after.name, "start it again");
        assert_eq!(store.triggers().len(), 1);

        assert!(store.remove_trigger("trg-1").unwrap());
        assert!(store.triggers().is_empty());
        assert!(!store.remove_trigger("trg-1").unwrap());
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
