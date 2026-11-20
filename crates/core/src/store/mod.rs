//! SQLite persistence: scans + snapshots now; findings, fix history and the
//! update cache are added by later migrations.

use crate::fix::{FixRecord, FixState};
use crate::snapshot::SystemSnapshot;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use ts_rs::TS;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("serialisation error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ScanStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}

impl ScanStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            "running" => Self::Running,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScanSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: ScanStatus,
    pub device_count: Option<u32>,
    pub warning_count: Option<u32>,
    pub error: Option<String>,
}

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
    CREATE TABLE scans (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        started_at    TEXT NOT NULL,
        finished_at   TEXT,
        status        TEXT NOT NULL,
        device_count  INTEGER,
        warning_count INTEGER,
        error         TEXT
    );
    CREATE TABLE snapshots (
        scan_id INTEGER PRIMARY KEY REFERENCES scans(id) ON DELETE CASCADE,
        json    TEXT NOT NULL
    );
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    "#,
    // v2
    r#"
    CREATE TABLE fix_history (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        created_at TEXT NOT NULL,
        state      TEXT NOT NULL,
        device_id  TEXT NOT NULL,
        rule_id    TEXT NOT NULL,
        json       TEXT NOT NULL
    );
    CREATE INDEX fix_history_device ON fix_history(device_id);
    "#,
];

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut s = Self { conn };
        s.migrate()?;
        Ok(s)
    }

    fn migrate(&mut self) -> Result<()> {
        let current: usize = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?
            as usize;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    pub fn begin_scan(&self) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO scans (started_at, status) VALUES (?1, ?2)",
            params![
                chrono::Utc::now().to_rfc3339(),
                ScanStatus::Running.as_str()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn finish_scan(
        &mut self,
        id: i64,
        status: ScanStatus,
        snapshot: Option<&SystemSnapshot>,
        error: Option<&str>,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE scans SET finished_at = ?2, status = ?3, device_count = ?4,
                              warning_count = ?5, error = ?6 WHERE id = ?1",
            params![
                id,
                chrono::Utc::now().to_rfc3339(),
                status.as_str(),
                snapshot.map(|s| s.devices.len() as i64),
                snapshot.map(|s| s.warnings.len() as i64),
                error,
            ],
        )?;
        if let Some(s) = snapshot {
            tx.execute(
                "INSERT OR REPLACE INTO snapshots (scan_id, json) VALUES (?1, ?2)",
                params![id, serde_json::to_string(s)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Mark scans left "running" by a crash/kill as failed.
    pub fn recover_interrupted(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE scans SET status = 'failed', error = 'interrupted' WHERE status = 'running'",
            [],
        )?)
    }

    pub fn list_scans(&self, limit: u32) -> Result<Vec<ScanSummary>> {
        let mut st = self.conn.prepare(
            "SELECT id, started_at, finished_at, status, device_count, warning_count, error
             FROM scans ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit], |r| {
            Ok(ScanSummary {
                id: r.get(0)?,
                started_at: r.get(1)?,
                finished_at: r.get(2)?,
                status: ScanStatus::parse(&r.get::<_, String>(3)?),
                device_count: r.get(4)?,
                warning_count: r.get(5)?,
                error: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn latest_snapshot(&self) -> Result<Option<(ScanSummary, SystemSnapshot)>> {
        let row: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT s.id, n.json FROM scans s JOIN snapshots n ON n.scan_id = s.id
                 WHERE s.status = 'completed' ORDER BY s.id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((id, json)) = row else {
            return Ok(None);
        };
        let summary = self
            .list_scans(u32::MAX)?
            .into_iter()
            .find(|s| s.id == id)
            .expect("scan row exists");
        Ok(Some((summary, serde_json::from_str(&json)?)))
    }

    fn state_str(s: FixState) -> Result<String> {
        Ok(serde_json::to_value(s)?.as_str().unwrap_or("unknown").to_string())
    }

    /// Inserts the record and returns its id (the stored JSON gets the id too).
    pub fn insert_fix(&self, r: &FixRecord) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO fix_history (created_at, state, device_id, rule_id, json)
             VALUES (?1, ?2, ?3, ?4, '{}')",
            params![r.created_at, Self::state_str(r.state)?, r.device_id, r.rule_id],
        )?;
        let id = self.conn.last_insert_rowid();
        let mut r = r.clone();
        r.id = id;
        self.update_fix(&r)?;
        Ok(id)
    }

    pub fn update_fix(&self, r: &FixRecord) -> Result<()> {
        self.conn.execute(
            "UPDATE fix_history SET state = ?2, json = ?3 WHERE id = ?1",
            params![r.id, Self::state_str(r.state)?, serde_json::to_string(r)?],
        )?;
        Ok(())
    }

    pub fn get_fix(&self, id: i64) -> Result<Option<FixRecord>> {
        let json: Option<String> = self
            .conn
            .query_row("SELECT json FROM fix_history WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    }

    /// Newest first.
    pub fn list_fixes(&self, limit: u32) -> Result<Vec<FixRecord>> {
        let mut st = self
            .conn
            .prepare("SELECT json FROM fix_history ORDER BY id DESC LIMIT ?1")?;
        let rows = st.query_map([limit], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for j in rows {
            out.push(serde_json::from_str(&j?)?);
        }
        Ok(out)
    }

    /// Fixes left pending by a crash or kill: their outcome is unknown.
    pub fn mark_pending_unknown(&self) -> Result<usize> {
        let pending = self
            .list_fixes(u32::MAX)?
            .into_iter()
            .filter(|r| r.state == FixState::Pending);
        let mut n = 0;
        for mut r in pending {
            r.state = FixState::Unknown;
            r.notes
                .push("Auriga was closed before this change finished; check the current value.".into());
            self.update_fix(&r)?;
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{MachineInfo, SNAPSHOT_SCHEMA_VERSION};

    fn snap() -> SystemSnapshot {
        SystemSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            collected_at: "2026-01-01T00:00:00+00:00".into(),
            machine: MachineInfo::default(),
            ..Default::default()
        }
    }

    #[test]
    fn v1_snapshot_json_still_loads() {
        let v1 = r#"{"schemaVersion":1,"collectedAt":"2026-01-01T00:00:00Z","machine":{},"devices":[],"warnings":[]}"#;
        let s: SystemSnapshot = serde_json::from_str(v1).unwrap();
        assert!(s.events.is_empty() && s.setupapi.is_empty() && s.event_window_days == 0);
    }

    #[test]
    fn migrate_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.db");
        let s = Store::open(&p).unwrap();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as i64);
        drop(s);
        let s = Store::open(&p).unwrap();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as i64);
    }

    #[test]
    fn scan_roundtrip_and_latest() {
        let mut s = Store::open_in_memory().unwrap();
        assert!(s.latest_snapshot().unwrap().is_none());
        let a = s.begin_scan().unwrap();
        s.finish_scan(a, ScanStatus::Completed, Some(&snap()), None)
            .unwrap();
        let b = s.begin_scan().unwrap();
        s.finish_scan(b, ScanStatus::Cancelled, None, None).unwrap();
        let (sum, _) = s.latest_snapshot().unwrap().unwrap();
        assert_eq!(
            sum.id, a,
            "cancelled scan must not replace last good snapshot"
        );
        assert_eq!(s.list_scans(10).unwrap().len(), 2);
    }

    #[test]
    fn interrupted_scans_recovered() {
        let s = Store::open_in_memory().unwrap();
        s.begin_scan().unwrap();
        assert_eq!(s.recover_interrupted().unwrap(), 1);
        assert_eq!(s.list_scans(1).unwrap()[0].status, ScanStatus::Failed);
    }
}
