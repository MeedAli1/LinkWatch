use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Arc;

use crate::engine::icmp::ProbeObservation;

#[derive(Clone)]
pub struct Database {
    pub conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TargetStat {
    pub target_id: String,
    pub target_label: String,
    pub host: String,
    pub total_probes: i64,
    pub success_count: i64,
    pub loss_percent: f64,
    pub avg_rtt_ms: f64,
    pub min_rtt_ms: f64,
    pub max_rtt_ms: f64,
    pub last_rtt_ms: Option<f64>,
    pub last_status: bool,
    pub last_seen_ms: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TimeSeriesBucket {
    pub bucket_start_ms: i64,
    pub bucket_end_ms: i64,
    pub total_probes: i64,
    pub success_count: i64,
    pub loss_count: i64,
    pub loss_percent: f64,
    pub avg_rtt_ms: Option<f64>,
    pub min_rtt_ms: Option<f64>,
    pub max_rtt_ms: Option<f64>,
    pub last_resolved_ip: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TargetTimeSeriesResult {
    pub target_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub bucket_size_ms: i64,
    pub buckets: Vec<TimeSeriesBucket>,
    pub recent_probes: Vec<ProbeObservation>,
    pub outages: Vec<OutageEvent>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OutageEvent {
    pub id: i64,
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    pub duration_seconds: Option<i64>,
    pub affected_targets: String,
    pub summary: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CustomTarget {
    pub id: i64,
    pub target_id: String,
    pub label: String,
    pub host: String,
    pub enabled: bool,
    pub created_ms: i64,
}

/// Retention & app settings stored in SQLite (key-value).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RetentionSettings {
    /// Whether automatic pruning is active.
    pub pruning_enabled: bool,
    /// Maximum age of probe_history rows in days. 0 = no age limit.
    pub max_age_days: u32,
    /// Maximum number of probe_history rows. 0 = no count limit.
    pub max_records: u64,
    /// Timestamp (ms) of last successful cleanup. None if never run.
    pub last_cleanup_ms: Option<i64>,
}

impl Default for RetentionSettings {
    fn default() -> Self {
        RetentionSettings {
            pruning_enabled: true,
            max_age_days: 7,
            max_records: 50_000,
            last_cleanup_ms: None,
        }
    }
}

/// Summary returned to the UI for the Settings panel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DbStats {
    pub probe_count: i64,
    pub outage_count: i64,
    pub custom_target_count: i64,
    pub db_size_bytes: u64,
    pub wal_size_bytes: u64,
    pub last_cleanup_ms: Option<i64>,
    pub oldest_probe_ms: Option<i64>,
}

/// Validation errors for new custom targets.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TargetValidationError {
    pub field: String,
    pub message: String,
}

pub fn normalize_host(raw: &str) -> String {
    let trimmed = raw.trim().to_lowercase();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed
    }
}

pub fn validate_target(label: &str, host: &str) -> Result<String, TargetValidationError> {
    let label = label.trim();
    if label.is_empty() {
        return Err(TargetValidationError { field: "label".into(), message: "Display name cannot be empty.".into() });
    }
    if label.len() > 64 {
        return Err(TargetValidationError { field: "label".into(), message: "Display name must be 64 characters or fewer.".into() });
    }
    let norm_host = normalize_host(host);
    if norm_host.is_empty() {
        return Err(TargetValidationError { field: "host".into(), message: "Address cannot be empty.".into() });
    }
    if norm_host.len() > 253 {
        return Err(TargetValidationError { field: "host".into(), message: "Address too long (max 253 characters).".into() });
    }
    let valid_chars = norm_host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'));
    if !valid_chars {
        return Err(TargetValidationError { field: "host".into(), message: "Address contains invalid characters. Use a valid IP or hostname.".into() });
    }
    if !norm_host.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(TargetValidationError { field: "host".into(), message: "Address must contain at least one alphanumeric character.".into() });
    }
    Ok(norm_host)
}

impl Database {
    pub fn init<P: AsRef<Path>>(db_path: P) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 268435456;
             PRAGMA cache_size = -8000;
             PRAGMA auto_vacuum = INCREMENTAL;
             -- Limit WAL auto-checkpoint so it doesn't balloon during bursts.
             PRAGMA wal_autocheckpoint = 1000;

             CREATE TABLE IF NOT EXISTS probe_history (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 target_id TEXT NOT NULL,
                 target_label TEXT NOT NULL,
                 host TEXT NOT NULL,
                 resolved_ip TEXT,
                 timestamp_ms INTEGER NOT NULL,
                 rtt_api_ms INTEGER,
                 rtt_measured_us INTEGER,
                 success INTEGER NOT NULL,
                 status_code INTEGER NOT NULL,
                 error_msg TEXT
             );

             CREATE INDEX IF NOT EXISTS idx_probe_target_time
             ON probe_history(target_id, timestamp_ms);

             -- Separate index used by age-based pruning (timestamp-only scan)
             CREATE INDEX IF NOT EXISTS idx_probe_timestamp
             ON probe_history(timestamp_ms);

             CREATE TABLE IF NOT EXISTS outages (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 start_ms INTEGER NOT NULL,
                 end_ms INTEGER,
                 duration_seconds INTEGER,
                 affected_targets TEXT NOT NULL,
                 summary TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS custom_targets (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 target_id TEXT NOT NULL UNIQUE,
                 label TEXT NOT NULL,
                 host TEXT NOT NULL,
                 host_normalized TEXT NOT NULL,
                 enabled INTEGER NOT NULL DEFAULT 1,
                 created_ms INTEGER NOT NULL
             );

             CREATE UNIQUE INDEX IF NOT EXISTS idx_custom_targets_host_norm
             ON custom_targets(host_normalized);

             -- Key-value settings store (migration: idempotent)
             CREATE TABLE IF NOT EXISTS app_settings (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
            ",
        )?;

        Ok(Database { conn: Arc::new(Mutex::new(conn)) })
    }

    // ─── WAL / maintenance ────────────────────────────────────────────────────

    /// Passive WAL checkpoint – does not block writers.
    pub fn checkpoint_passive(&self) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock();
        conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")?;
        Ok(())
    }

    /// Full truncating WAL checkpoint – used at shutdown.
    pub fn checkpoint(&self) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    /// Current page count × page size = approximate file size in bytes.
    pub fn db_page_size_bytes(&self) -> u64 {
        let conn = self.conn.lock();
        let pages: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap_or(0);
        let page_sz: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap_or(4096);
        (pages * page_sz) as u64
    }

    // ─── Probe history ────────────────────────────────────────────────────────

    pub fn insert_observation(&self, obs: &ProbeObservation) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO probe_history
             (target_id, target_label, host, resolved_ip, timestamp_ms, rtt_api_ms, rtt_measured_us, success, status_code, error_msg)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                obs.target_id, obs.target_label, obs.host, obs.resolved_ip,
                obs.timestamp_ms, obs.rtt_api_ms, obs.rtt_measured_us,
                if obs.success { 1 } else { 0 },
                obs.status_code, obs.error_msg,
            ],
        )?;
        Ok(())
    }

    pub fn get_recent_history(&self, limit: usize) -> Result<Vec<ProbeObservation>, rusqlite::Error> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT target_id, target_label, host, resolved_ip, timestamp_ms,
                    rtt_api_ms, rtt_measured_us, success, status_code, error_msg
             FROM probe_history ORDER BY timestamp_ms DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let s: i32 = row.get(7)?;
            Ok(ProbeObservation {
                target_id: row.get(0)?, target_label: row.get(1)?,
                host: row.get(2)?, resolved_ip: row.get(3)?,
                timestamp_ms: row.get(4)?, rtt_api_ms: row.get(5)?,
                rtt_measured_us: row.get(6)?, success: s == 1,
                status_code: row.get(8)?, error_msg: row.get(9)?,
            })
        })?;
        rows.collect()
    }

    pub fn get_target_stats(&self) -> Result<Vec<TargetStat>, rusqlite::Error> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT target_id, target_label, host,
                    COUNT(*) as total,
                    SUM(CASE WHEN success=1 THEN 1 ELSE 0 END) as successes,
                    AVG(CASE WHEN success=1 THEN rtt_measured_us/1000.0 ELSE NULL END) as avg_rtt,
                    MIN(CASE WHEN success=1 THEN rtt_measured_us/1000.0 ELSE NULL END) as min_rtt,
                    MAX(CASE WHEN success=1 THEN rtt_measured_us/1000.0 ELSE NULL END) as max_rtt,
                    MAX(timestamp_ms) as last_seen
             FROM probe_history
             GROUP BY target_id, target_label, host",
        )?;
        let rows = stmt.query_map([], |row| {
            let total: i64 = row.get(3)?;
            let successes: i64 = row.get::<_, Option<i64>>(4)?.unwrap_or(0);
            let loss_percent = if total > 0 { ((total - successes) as f64 / total as f64) * 100.0 } else { 0.0 };
            Ok(TargetStat {
                target_id: row.get(0)?, target_label: row.get(1)?,
                host: row.get(2)?, total_probes: total,
                success_count: successes, loss_percent,
                avg_rtt_ms: row.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                min_rtt_ms: row.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                max_rtt_ms: row.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
                last_rtt_ms: None, last_status: false,
                last_seen_ms: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
            })
        })?;
        rows.collect()
    }
    /// Query aggregated time-series buckets for a target between start_ms and end_ms.
    /// Also returns fine-grained individual recent probes (up to 120) and relevant outages.
    pub fn get_target_timeseries(
        &self,
        target_id: &str,
        start_ms: i64,
        end_ms: i64,
        bucket_size_ms: i64,
    ) -> Result<TargetTimeSeriesResult, rusqlite::Error> {
        let conn = self.conn.lock();
        let bucket_sz = if bucket_size_ms <= 0 { 60_000 } else { bucket_size_ms };

        // 1. Aggregated query using SQLite integer division to bucket by timestamp
        // Uses idx_probe_target_time (target_id, timestamp_ms)
        let mut stmt = conn.prepare(
            "SELECT
                (timestamp_ms / ?1) * ?1 as b_start,
                COUNT(*) as total,
                SUM(CASE WHEN success=1 THEN 1 ELSE 0 END) as successes,
                AVG(CASE WHEN success=1 THEN rtt_measured_us/1000.0 ELSE NULL END) as avg_rtt,
                MIN(CASE WHEN success=1 THEN rtt_measured_us/1000.0 ELSE NULL END) as min_rtt,
                MAX(CASE WHEN success=1 THEN rtt_measured_us/1000.0 ELSE NULL END) as max_rtt,
                MAX(resolved_ip) as last_ip
             FROM probe_history
             WHERE target_id = ?2 AND timestamp_ms >= ?3 AND timestamp_ms <= ?4
             GROUP BY b_start
             ORDER BY b_start ASC",
        )?;

        let rows = stmt.query_map(params![bucket_sz, target_id, start_ms, end_ms], |row| {
            let b_start: i64 = row.get(0)?;
            let total: i64 = row.get(1)?;
            let successes: i64 = row.get::<_, Option<i64>>(2)?.unwrap_or(0);
            let loss_count = total - successes;
            let loss_percent = if total > 0 { (loss_count as f64 / total as f64) * 100.0 } else { 0.0 };
            Ok(TimeSeriesBucket {
                bucket_start_ms: b_start,
                bucket_end_ms: b_start + bucket_sz,
                total_probes: total,
                success_count: successes,
                loss_count,
                loss_percent,
                avg_rtt_ms: row.get::<_, Option<f64>>(3)?,
                min_rtt_ms: row.get::<_, Option<f64>>(4)?,
                max_rtt_ms: row.get::<_, Option<f64>>(5)?,
                last_resolved_ip: row.get::<_, Option<String>>(6)?,
            })
        })?;

        let buckets = rows.collect::<Result<Vec<TimeSeriesBucket>, rusqlite::Error>>()?;

        // 2. Fetch individual recent probe records within window (limit 120) for exact tooltip inspection
        let mut probe_stmt = conn.prepare(
            "SELECT target_id, target_label, host, resolved_ip, timestamp_ms,
                    rtt_api_ms, rtt_measured_us, success, status_code, error_msg
             FROM probe_history
             WHERE target_id = ?1 AND timestamp_ms >= ?2 AND timestamp_ms <= ?3
             ORDER BY timestamp_ms DESC LIMIT 120",
        )?;

        let probe_rows = probe_stmt.query_map(params![target_id, start_ms, end_ms], |row| {
            let s: i32 = row.get(7)?;
            Ok(ProbeObservation {
                target_id: row.get(0)?, target_label: row.get(1)?,
                host: row.get(2)?, resolved_ip: row.get(3)?,
                timestamp_ms: row.get(4)?, rtt_api_ms: row.get(5)?,
                rtt_measured_us: row.get(6)?, success: s == 1,
                status_code: row.get(8)?, error_msg: row.get(9)?,
            })
        })?;

        let recent_probes = probe_rows.collect::<Result<Vec<ProbeObservation>, rusqlite::Error>>()?;

        // 3. Fetch overlapping outages in this time window
        let mut outage_stmt = conn.prepare(
            "SELECT id, start_ms, end_ms, duration_seconds, affected_targets, summary
             FROM outages
             WHERE (end_ms IS NULL OR end_ms >= ?1) AND start_ms <= ?2
             ORDER BY start_ms ASC LIMIT 50",
        )?;

        let outage_rows = outage_stmt.query_map(params![start_ms, end_ms], |row| {
            Ok(OutageEvent {
                id: row.get(0)?, start_ms: row.get(1)?,
                end_ms: row.get(2)?, duration_seconds: row.get(3)?,
                affected_targets: row.get(4)?, summary: row.get(5)?,
            })
        })?;

        let outages = outage_rows.collect::<Result<Vec<OutageEvent>, rusqlite::Error>>()?;

        Ok(TargetTimeSeriesResult {
            target_id: target_id.to_string(),
            start_ms,
            end_ms,
            bucket_size_ms: bucket_sz,
            buckets,
            recent_probes,
            outages,
        })
    }

    pub fn count_probes(&self) -> Result<i64, rusqlite::Error> {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM probe_history", [], |r| r.get(0))
    }

    pub fn oldest_probe_ms(&self) -> Result<Option<i64>, rusqlite::Error> {
        let conn = self.conn.lock();
        conn.query_row("SELECT MIN(timestamp_ms) FROM probe_history", [], |r| r.get(0))
    }

    // ─── Outages ─────────────────────────────────────────────────────────────

    pub fn record_outage(&self, start_ms: i64, end_ms: Option<i64>, affected: &str, summary: &str) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock();
        let duration = end_ms.map(|e| (e - start_ms) / 1000);
        conn.execute(
            "INSERT INTO outages (start_ms, end_ms, duration_seconds, affected_targets, summary) VALUES (?1,?2,?3,?4,?5)",
            params![start_ms, end_ms, duration, affected, summary],
        )?;
        Ok(())
    }

    pub fn get_outages(&self, limit: usize) -> Result<Vec<OutageEvent>, rusqlite::Error> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, start_ms, end_ms, duration_seconds, affected_targets, summary
             FROM outages ORDER BY start_ms DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(OutageEvent {
                id: row.get(0)?, start_ms: row.get(1)?,
                end_ms: row.get(2)?, duration_seconds: row.get(3)?,
                affected_targets: row.get(4)?, summary: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    // ─── Custom targets ───────────────────────────────────────────────────────

    pub fn list_custom_targets(&self) -> Result<Vec<CustomTarget>, rusqlite::Error> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, target_id, label, host, enabled, created_ms FROM custom_targets ORDER BY created_ms ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let en: i32 = row.get(4)?;
            Ok(CustomTarget {
                id: row.get(0)?, target_id: row.get(1)?,
                label: row.get(2)?, host: row.get(3)?,
                enabled: en != 0, created_ms: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn insert_custom_target(&self, target_id: &str, label: &str, host: &str, host_normalized: &str, created_ms: i64) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO custom_targets (target_id, label, host, host_normalized, enabled, created_ms) VALUES (?1,?2,?3,?4,1,?5)",
            params![target_id, label, host, host_normalized, created_ms],
        )?;
        Ok(())
    }

    pub fn set_custom_target_enabled(&self, target_id: &str, enabled: bool) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock();
        conn.execute("UPDATE custom_targets SET enabled=?1 WHERE target_id=?2", params![if enabled { 1 } else { 0 }, target_id])?;
        Ok(())
    }

    pub fn delete_custom_target(&self, target_id: &str) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM custom_targets WHERE target_id=?1", params![target_id])?;
        Ok(())
    }

    pub fn custom_target_host_exists(&self, host_normalized: &str) -> Result<bool, rusqlite::Error> {
        let conn = self.conn.lock();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM custom_targets WHERE host_normalized=?1", params![host_normalized], |r| r.get(0))?;
        Ok(count > 0)
    }

    // ─── App settings (key-value) ─────────────────────────────────────────────

    fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock();
        conn.query_row("SELECT value FROM app_settings WHERE key=?1", params![key], |r| r.get(0)).ok()
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn load_retention_settings(&self) -> RetentionSettings {
        RetentionSettings {
            pruning_enabled: self.get_setting("pruning_enabled")
                .and_then(|v| v.parse::<u8>().ok())
                .map(|n| n != 0)
                .unwrap_or(true),
            max_age_days: self.get_setting("max_age_days")
                .and_then(|v| v.parse().ok())
                .unwrap_or(7),
            max_records: self.get_setting("max_records")
                .and_then(|v| v.parse().ok())
                .unwrap_or(50_000),
            last_cleanup_ms: self.get_setting("last_cleanup_ms")
                .and_then(|v| v.parse().ok()),
        }
    }

    pub fn save_retention_settings(&self, s: &RetentionSettings) -> Result<(), rusqlite::Error> {
        self.set_setting("pruning_enabled", if s.pruning_enabled { "1" }
 else { "0" })?;
        self.set_setting("max_age_days", &s.max_age_days.to_string())?;
        self.set_setting("max_records", &s.max_records.to_string())?;
        if let Some(ts) = s.last_cleanup_ms {
            self.set_setting("last_cleanup_ms", &ts.to_string())?;
        }
        Ok(())
    }

    pub fn get_start_minimized(&self) -> bool {
        self.get_setting("start_minimized")
            .and_then(|v| v.parse::<u8>().ok())
            .map(|n| n != 0)
            .unwrap_or(false)
    }

    pub fn set_start_minimized(&self, enabled: bool) -> Result<(), rusqlite::Error> {
        self.set_setting("start_minimized", if enabled { "1" } else { "0" })
    }

    pub fn get_probe_interval_ms(&self) -> Option<u64> {
        self.get_setting("probe_interval_ms")
            .and_then(|v| v.parse().ok())
    }

    pub fn set_probe_interval_ms(&self, ms: u64) -> Result<(), rusqlite::Error> {
        self.set_setting("probe_interval_ms", &ms.to_string())
    }

    // ─── Pruning ──────────────────────────────────────────────────────────────

    /// Delete probe_history rows older than `max_age_ms` milliseconds.
    /// Uses batched deletes of 1000 rows to avoid long locks.
    /// Returns number of rows deleted.
    pub fn prune_by_age(&self, cutoff_ms: i64, batch: usize) -> Result<usize, rusqlite::Error> {
        let mut total = 0usize;
        loop {
            let conn = self.conn.lock();
            let deleted = conn.execute(
                "DELETE FROM probe_history WHERE id IN (
                    SELECT id FROM probe_history
                    WHERE timestamp_ms < ?1
                    ORDER BY timestamp_ms ASC
                    LIMIT ?2
                )",
                params![cutoff_ms, batch as i64],
            )?;
            drop(conn);
            total += deleted;
            if deleted < batch { break; }
        }
        Ok(total)
    }

    /// Delete oldest probe_history rows until count <= max_records.
    /// Uses batched deletes of 1000 rows.
    /// Returns number of rows deleted.
    pub fn prune_by_count(&self, max_records: usize, batch: usize) -> Result<usize, rusqlite::Error> {
        let mut total = 0usize;
        loop {
            let count: i64 = {
                let conn = self.conn.lock();
                conn.query_row("SELECT COUNT(*) FROM probe_history", [], |r| r.get(0))?
            };
            let excess = count as usize;
            if excess <= max_records { break; }
            let to_delete = (excess - max_records).min(batch);
            let conn = self.conn.lock();
            let deleted = conn.execute(
                "DELETE FROM probe_history WHERE id IN (
                    SELECT id FROM probe_history
                    ORDER BY timestamp_ms ASC
                    LIMIT ?1
                )",
                params![to_delete as i64],
            )?;
            drop(conn);
            total += deleted;
            if deleted == 0 { break; }
        }
        Ok(total)
    }

    /// Also prune outage records older than cutoff (keep last 500 at minimum).
    pub fn prune_outages(&self, cutoff_ms: i64) -> Result<usize, rusqlite::Error> {
        // Always keep at least 500 most-recent outages regardless of age.
        let count: i64 = {
            let conn = self.conn.lock();
            conn.query_row("SELECT COUNT(*) FROM outages", [], |r| r.get(0))?
        };
        if count <= 500 { return Ok(0); }
        let conn = self.conn.lock();
        let deleted = conn.execute(
            "DELETE FROM outages WHERE id IN (
                SELECT id FROM outages
                WHERE start_ms < ?1
                ORDER BY start_ms ASC
                LIMIT ?2
            )",
            params![cutoff_ms, (count - 500) as i64],
        )?;
        Ok(deleted)
    }

    /// Gather UI statistics without blocking monitoring.
    pub fn get_db_stats(&self, wal_path: Option<&std::path::Path>) -> DbStats {
        let probe_count = self.count_probes().unwrap_or(0);
        let oldest_probe_ms = self.oldest_probe_ms().unwrap_or(None);
        let outage_count = {
            let conn = self.conn.lock();
            conn.query_row("SELECT COUNT(*) FROM outages", [], |r| r.get(0)).unwrap_or(0)
        };
        let custom_target_count = {
            let conn = self.conn.lock();
            conn.query_row("SELECT COUNT(*) FROM custom_targets", [], |r| r.get(0)).unwrap_or(0)
        };
        let db_size_bytes = self.db_page_size_bytes();
        let wal_size_bytes = wal_path
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .unwrap_or(0);
        let last_cleanup_ms = self.get_setting("last_cleanup_ms")
            .and_then(|v| v.parse().ok());

        DbStats {
            probe_count, outage_count, custom_target_count,
            db_size_bytes, wal_size_bytes,
            last_cleanup_ms, oldest_probe_ms,
        }
    }
}
