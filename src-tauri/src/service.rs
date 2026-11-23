use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::diagnostics::engine::{DiagnosticEngine, DiagnosticFinding};
use crate::engine::gateway::get_default_gateway;
use crate::engine::icmp::{ProbeObservation, WinIcmpPinger};
use crate::storage::db::{CustomTarget, Database, OutageEvent, TargetStat};
use crate::storage::retention::RetentionService;

/// How many of the most recent ring-buffer observations feed the diagnostic engine.
/// At the default 1.5 s interval with ~4 targets this covers the last ~45 s (~30 cycles),
/// so one transient timeout cannot flip the verdict the way a single cycle could.
const DIAG_WINDOW_OBS: usize = 120;

/// Escape user-controlled text before interpolating into HTML exports. Target
/// labels and outage summaries reach the report verbatim from the DB and would
/// otherwise execute when the exported file is opened in a browser.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// RFC-4180 quoting: doubles embedded quotes and wraps the field.
fn csv_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MonitorTarget {
    pub id: String,
    pub label: String,
    pub host: String,
    pub is_gateway: bool,
    /// true = built-in (gateway, DNS, etc.); false = user-defined custom target
    pub is_builtin: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppStateSnapshot {
    pub is_monitoring: bool,
    pub storage_degraded: bool,
    pub active_gateway: Option<String>,
    pub latest_observations: Vec<ProbeObservation>,
    pub stats: Vec<TargetStat>,
    pub diagnostic: DiagnosticFinding,
    pub recent_outages: Vec<OutageEvent>,
    pub custom_targets: Vec<CustomTarget>,
}

pub struct MonitorService {
    pinger: Arc<WinIcmpPinger>,
    db: Arc<Database>,
    is_running: Arc<AtomicBool>,
    targets: Arc<Mutex<Vec<MonitorTarget>>>,
    recent_observations: Arc<Mutex<VecDeque<ProbeObservation>>>,
    latest_diagnostic: Arc<Mutex<DiagnosticFinding>>,
    in_outage: Arc<AtomicBool>,
    outage_start_ms: Arc<Mutex<Option<i64>>>,
    worker_handle: Mutex<Option<JoinHandle<()>>>,
    retention: RetentionService,
    /// Probe cycle interval in milliseconds (500 – 30_000). Stored atomically for lock-free reads.
    probe_interval_ms: Arc<AtomicU64>,
    /// Consecutive probe cycles that failed to write history. Live charts
    /// stay fed from the in-memory ring, so a persistently failing DB is
    /// otherwise silent; once the count passes the threshold below,
    /// storage_degraded latches and the UI and tray report it. A fully
    /// successful cycle resets both.
    db_write_fail_streak: Arc<AtomicU64>,
    storage_degraded: Arc<AtomicBool>,
    /// Set once by shutdown(). Blocks start() forever after: a Resume click
    /// flipping is_running back to true mid-shutdown-join resurrects the
    /// worker the shutdown thread is waiting on, and the app never exits.
    shutdown_requested: Arc<AtomicBool>,
    /// Set while stop() is joining the worker. Blocks start() for the
    /// duration of the join: a Resume click landing mid-join would flip
    /// is_running back to true and resurrect the very worker being joined,
    /// leaving two probe loops running against one database.
    joining: Arc<AtomicBool>,
}

/// Consecutive failed probe cycles before storage is declared degraded. One
/// cycle writes one row per active target (4 built-ins plus custom).
const DB_WRITE_FAIL_DEGRADE_THRESHOLD: u64 = 4;

/// Applies one probe cycle's history-write outcome to the degrade counters.
/// A fully successful cycle resets both; a cycle with any failed write
/// advances the streak and latches the flag once the threshold is reached.
fn record_cycle_writes(
    streak: &std::sync::atomic::AtomicU64,
    degraded: &std::sync::atomic::AtomicBool,
    cycle_failures: usize,
) {
    if cycle_failures == 0 {
        streak.store(0, Ordering::SeqCst);
        degraded.store(false, Ordering::SeqCst);
    } else if streak.fetch_add(1, Ordering::SeqCst) + 1 >= DB_WRITE_FAIL_DEGRADE_THRESHOLD {
        degraded.store(true, Ordering::SeqCst);
    }
}

impl MonitorService {
    /// True once the DB has refused that many probe cycles in a row:
    /// telemetry is still visible live but no longer being recorded.
    pub fn storage_degraded(&self) -> bool {
        self.storage_degraded.load(Ordering::SeqCst)
    }
    pub fn new(db: Database, db_path: std::path::PathBuf) -> Result<Self, String> {
        let pinger = WinIcmpPinger::new()?;

        // Built-in targets
        let mut initial_targets = vec![
            MonitorTarget {
                id: "dns_primary".into(),
                label: "Primary DNS (Cloudflare)".into(),
                host: "1.1.1.1".into(),
                is_gateway: false,
                is_builtin: true,
            },
            MonitorTarget {
                id: "public_ip".into(),
                label: "Public DNS (Google)".into(),
                host: "8.8.8.8".into(),
                is_gateway: false,
                is_builtin: true,
            },
            MonitorTarget {
                id: "web_target".into(),
                label: "Web Target (one.one.one.one)".into(),
                host: "one.one.one.one".into(),
                is_gateway: false,
                is_builtin: true,
            },
        ];

        // Detect default gateway
        if let Some(gw) = get_default_gateway() {
            initial_targets.insert(0, MonitorTarget {
                id: "gateway".into(),
                label: "Default Router Gateway".into(),
                host: gw,
                is_gateway: true,
                is_builtin: true,
            });
        }

        // Restore enabled custom targets from SQLite
        let custom = db.list_custom_targets().unwrap_or_default();
        for ct in &custom {
            if ct.enabled {
                initial_targets.push(MonitorTarget {
                    id: ct.target_id.clone(),
                    label: ct.label.clone(),
                    host: ct.host.clone(),
                    is_gateway: false,
                    is_builtin: false,
                });
            }
        }

        let initial_diag = DiagnosticEngine::analyze(&[]);

        let db_arc = Arc::new(db);
        let retention = RetentionService::new(Arc::clone(&db_arc), db_path);

        // Load saved probe interval or fall back to 1500 ms
        let saved_interval = db_arc
            .get_probe_interval_ms()
            .unwrap_or(1500)
            .clamp(500, 30_000);

        Ok(MonitorService {
            pinger: Arc::new(pinger),
            db: db_arc,
            is_running: Arc::new(AtomicBool::new(false)),
            targets: Arc::new(Mutex::new(initial_targets)),
            recent_observations: Arc::new(Mutex::new(VecDeque::with_capacity(500))),
            latest_diagnostic: Arc::new(Mutex::new(initial_diag)),
            in_outage: Arc::new(AtomicBool::new(false)),
            outage_start_ms: Arc::new(Mutex::new(None)),
            worker_handle: Mutex::new(None),
            retention,
            probe_interval_ms: Arc::new(AtomicU64::new(saved_interval as u64)),
            db_write_fail_streak: Arc::new(AtomicU64::new(0)),
            storage_degraded: Arc::new(AtomicBool::new(false)),
            shutdown_requested: Arc::new(AtomicBool::new(false)),
            joining: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    pub fn start(&self) {
        if self.shutdown_requested.load(Ordering::SeqCst) {
            return; // an exit was requested: restarting here would deadlock the shutdown join
        }
        if self.joining.load(Ordering::SeqCst) {
            return; // a stop is mid-join: restarting now would resurrect that worker
        }
        if self.is_running.swap(true, Ordering::SeqCst) {
            return; // already running – single-loop guarantee
        }

        // Start background retention pruning on first monitoring start
        self.retention.start();

        let pinger = Arc::clone(&self.pinger);
        let db = Arc::clone(&self.db);
        let is_running = Arc::clone(&self.is_running);
        let targets = Arc::clone(&self.targets);
        let recent_obs = Arc::clone(&self.recent_observations);
        let latest_diag = Arc::clone(&self.latest_diagnostic);
        let in_outage = Arc::clone(&self.in_outage);
        let outage_start_ms = Arc::clone(&self.outage_start_ms);
        let probe_interval_ms = Arc::clone(&self.probe_interval_ms);
        let db_write_fail_streak = Arc::clone(&self.db_write_fail_streak);
        let storage_degraded = Arc::clone(&self.storage_degraded);

        let handle = thread::spawn(move || {
            while is_running.load(Ordering::SeqCst) {
                // Dynamically refresh gateway host if it changed
                {
                    let mut tgts = targets.lock();
                    if let Some(gw) = get_default_gateway() {
                        let has_gw = tgts.iter().any(|t| t.is_gateway);
                        if !has_gw {
                            tgts.insert(0, MonitorTarget {
                                id: "gateway".into(), label: "Default Router Gateway".into(),
                                host: gw, is_gateway: true, is_builtin: true,
                            });
                        } else if let Some(first) = tgts.iter_mut().find(|t| t.is_gateway) {
                            first.host = gw;
                        }
                    }
                }

                let current_targets = targets.lock().clone();

                // ── Parallel probe fan-out ────────────────────────────────────
                // Each target gets its own thread; all complete within 1×timeout_ms.
                // Results are ordered to match current_targets order for determinism.
                let mut indexed_obs: Vec<(usize, ProbeObservation)> = {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let running_flag = Arc::clone(&is_running);

                    thread::scope(|s| {
                        for (idx, t) in current_targets.iter().enumerate() {
                            if !running_flag.load(Ordering::SeqCst) { break; }
                            let tx = tx.clone();
                            let pinger = Arc::clone(&pinger);
                            let id = t.id.clone();
                            let label = t.label.clone();
                            let host = t.host.clone();
                            s.spawn(move || {
                                let obs = pinger.ping(&id, &label, &host, 1200);
                                let _ = tx.send((idx, obs));
                            });
                        }
                        // Drop the original tx so rx.iter() terminates when all spawned threads finish
                        drop(tx);
                        rx.into_iter().collect()
                    })
                };
                indexed_obs.sort_by_key(|(i, _)| *i);
                let cycle_obs: Vec<ProbeObservation> = indexed_obs.into_iter().map(|(_, o)| o).collect();

                // Persist to DB. A persistently failing DB must not stay
                // silent: the ring buffer keeps the charts alive while every
                // observation is dropped, so record_cycle_writes latches a
                // degrade flag the UI and tray report. One failed cycle is
                // counted once however many targets failed to write.
                let cycle_write_failures = cycle_obs
                    .iter()
                    .filter(|obs| db.insert_observation(obs).is_err())
                    .count();
                record_cycle_writes(
                    &db_write_fail_streak,
                    &storage_degraded,
                    cycle_write_failures,
                );

                // Update ring buffer
                {
                    let mut buf = recent_obs.lock();
                    for obs in &cycle_obs {
                        if buf.len() >= 500 { buf.pop_front(); }
                        buf.push_back(obs.clone());
                    }
                }

                // Diagnostic over the rolling window (not just this cycle)
                let diag_window: Vec<ProbeObservation> = {
                    let buf = recent_obs.lock();
                    buf.iter().rev().take(DIAG_WINDOW_OBS).rev().cloned().collect()
                };
                *latest_diag.lock() = DiagnosticEngine::analyze(&diag_window);

                // Outage tracking (external targets only)
                let all_ext_failing = !cycle_obs.is_empty()
                    && cycle_obs.iter().filter(|o| o.target_id != "gateway").all(|o| !o.success);

                if all_ext_failing {
                    if !in_outage.load(Ordering::SeqCst) {
                        in_outage.store(true, Ordering::SeqCst);
                        *outage_start_ms.lock() = Some(chrono::Utc::now().timestamp_millis());
                    }
                } else if in_outage.load(Ordering::SeqCst) {
                    in_outage.store(false, Ordering::SeqCst);
                    if let Some(s) = outage_start_ms.lock().take() {
                        let _ = db.record_outage(s, Some(chrono::Utc::now().timestamp_millis()),
                            "All Public Targets", "Connection restored after outage.");
                    }
                }

                // Sleep for configured interval, interruptible at 100 ms resolution
                let interval_ms = probe_interval_ms.load(Ordering::Relaxed);
                let ticks = (interval_ms / 100).max(1);
                for _ in 0..ticks {
                    if !is_running.load(Ordering::SeqCst) { break; }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        });

        *self.worker_handle.lock() = Some(handle);
    }

    pub fn stop_within(&self, wait: Duration) -> bool {
        self.is_running.store(false, Ordering::SeqCst);
        let handle = self.worker_handle.lock().take();
        let Some(h) = handle else {
            return true; // no worker running
        };
        // std has no timed join, so a detached waiter owns the handle; on
        // timeout the worker keeps running and cannot be joined twice.
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = h.join();
            let _ = tx.send(());
        });
        match rx.recv_timeout(wait) {
            Ok(()) => true,
            // Hung getaddrinfo: the worker outlives the wait and keeps
            // running detached. The caller (shutdown) must skip DB work
            // while it is alive; shutdown_requested already blocks start().
            Err(_) => false,
        }
    }

    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
        let handle = self.worker_handle.lock().take();
        if let Some(h) = handle {
            self.joining.store(true, Ordering::SeqCst);
            // Probe cycles are hard-bounded (resolve wait + ping timeout per
            // target, all parallel), so this join returns within about one
            // cycle. stop_within remains for the exit path that cannot wait
            // even that long on a misbehaving OS call.
            let _ = h.join();
            self.joining.store(false, Ordering::SeqCst);
        }
    }

    pub fn shutdown(&self) {
        // Blocks start() from this instant; a later Resume must not resurrect
        // the worker this shutdown is waiting on.
        self.shutdown_requested.store(true, Ordering::SeqCst);
        self.is_running.store(false, Ordering::SeqCst);
        self.retention.stop();
        let joined = self.stop_within(Duration::from_millis(2500));
        // Closing the outage row and checkpointing the WAL needs the DB mutex
        // the worker may still hold mid-write, so only do it once it is done.
        if joined {
            self.persist_shutdown_state();
        }
    }

    fn persist_shutdown_state(&self) {
        // Close any open outage
        if self.in_outage.load(Ordering::SeqCst) {
            if let Some(s) = self.outage_start_ms.lock().take() {
                let _ = self.db.record_outage(s, Some(chrono::Utc::now().timestamp_millis()),
                    "All Public Targets", "Application shutdown during outage.");
            }
        }
        let _ = self.db.checkpoint_passive();
    }

    // ─── Custom target management ─────────────────────────────────────────────

    /// Add a new custom target. Returns the created CustomTarget or a validation error.
    pub fn add_custom_target(&self, label: &str, host: &str)
        -> Result<CustomTarget, String>
    {
        // Validate input
        let norm_host = crate::storage::db::validate_target(label, host)
            .map_err(|e| e.message)?;

        // Check built-in duplicate
        {
            let tgts = self.targets.lock();
            if let Some(existing) = tgts.iter().find(|t| {
                crate::storage::db::normalize_host(&t.host) == norm_host
            }) {
                return Err(format!(
                    "\"{}\" is already being monitored as \"{}\".",
                    norm_host, existing.label
                ));
            }
        }

        // Check DB duplicate
        if self.db.custom_target_host_exists(&norm_host).map_err(|e| e.to_string())? {
            return Err(format!("\"{}\" is already a saved custom target.", norm_host));
        }

        // Generate a stable unique slug from timestamp
        let now_ms = chrono::Utc::now().timestamp_millis();
        let target_id = format!("custom_{:x}", now_ms as u64);

        self.db.insert_custom_target(&target_id, label.trim(), host.trim(), &norm_host, now_ms)
            .map_err(|e| e.to_string())?;

        // Add to live target list immediately (monitoring picks it up on next cycle)
        {
            let mut tgts = self.targets.lock();
            tgts.push(MonitorTarget {
                id: target_id.clone(),
                label: label.trim().to_string(),
                host: host.trim().to_string(),
                is_gateway: false,
                is_builtin: false,
            });
        }

        Ok(CustomTarget {
            id: 0, // actual row id not critical here
            target_id,
            label: label.trim().to_string(),
            host: host.trim().to_string(),
            enabled: true,
            created_ms: now_ms,
        })
    }

    /// Enable or disable a custom target.
    pub fn set_custom_target_enabled(&self, target_id: &str, enabled: bool) -> Result<(), String> {
        self.db.set_custom_target_enabled(target_id, enabled)
            .map_err(|e| e.to_string())?;

        let mut tgts = self.targets.lock();
        if enabled {
            // Re-add to active targets if not already present
            if !tgts.iter().any(|t| t.id == target_id) {
                // Look up details from DB
                let customs = self.db.list_custom_targets().map_err(|e| e.to_string())?;
                if let Some(ct) = customs.iter().find(|c| c.target_id == target_id) {
                    tgts.push(MonitorTarget {
                        id: ct.target_id.clone(), label: ct.label.clone(),
                        host: ct.host.clone(), is_gateway: false, is_builtin: false,
                    });
                }
            }
        } else {
            tgts.retain(|t| t.id != target_id || t.is_builtin);
        }
        Ok(())
    }

    /// Remove a custom target permanently. Historical probe data is preserved.
    pub fn remove_custom_target(&self, target_id: &str) -> Result<(), String> {
        // Guard: cannot remove built-in targets
        {
            let tgts = self.targets.lock();
            if let Some(t) = tgts.iter().find(|t| t.id == target_id) {
                if t.is_builtin {
                    return Err("Built-in targets cannot be removed.".into());
                }
            }
        }
        self.db.delete_custom_target(target_id).map_err(|e| e.to_string())?;
        self.targets.lock().retain(|t| t.id != target_id);
        self.recent_observations.lock().retain(|o| o.target_id != target_id);
        Ok(())
    }

    // ─── Snapshots & exports ──────────────────────────────────────────────────

    pub fn get_snapshot(&self) -> AppStateSnapshot {
        let is_monitoring = self.is_running.load(Ordering::SeqCst);
        let targets = self.targets.lock();
        let active_gateway = targets.iter().find(|t| t.is_gateway).map(|t| t.host.clone());
        let active_target_ids: Vec<String> = targets.iter().map(|t| t.id.clone()).collect();
        drop(targets);

        let latest_observations = self.recent_observations.lock().iter().cloned().collect();
        let mut stats = self.db.get_target_stats().unwrap_or_default();
        stats.retain(|s| active_target_ids.contains(&s.target_id));

        let diagnostic = self.latest_diagnostic.lock().clone();
        let recent_outages = self.db.get_outages(20).unwrap_or_default();
        let custom_targets = self.db.list_custom_targets().unwrap_or_default();

        AppStateSnapshot {
            is_monitoring, storage_degraded: self.storage_degraded.load(Ordering::SeqCst),
            active_gateway, latest_observations,
            stats, diagnostic, recent_outages, custom_targets,
        }
    }

    pub fn export_csv(&self) -> Result<String, String> {
        let history = self.db.get_recent_history(1000).map_err(|e| e.to_string())?;
        let mut csv = String::from("id,target_id,target_label,host,resolved_ip,timestamp_ms,rtt_measured_ms,success,error_msg\n");
        for (i, obs) in history.iter().enumerate() {
            let rtt_ms = obs.rtt_measured_us
                .map(|u| format!("{:.2}", u as f64 / 1000.0))
                .unwrap_or_default();
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{}\n",
                i + 1,
                csv_quote(&obs.target_id),
                csv_quote(&obs.target_label),
                csv_quote(&obs.host),
                csv_quote(obs.resolved_ip.as_deref().unwrap_or("")),
                obs.timestamp_ms, rtt_ms,
                if obs.success { 1 } else { 0 },
                csv_quote(obs.error_msg.as_deref().unwrap_or("")),
            ));
        }
        Ok(csv)
    }

    pub fn export_html_report(&self) -> Result<String, String> {
        let stats = self.db.get_target_stats().unwrap_or_default();
        let outages = self.db.get_outages(50).unwrap_or_default();
        let diag = self.latest_diagnostic.lock().clone();
        let generated_at = chrono::Utc::now().to_rfc3339();

        let mut html = format!(
            r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>LinkWatch Network Diagnostics &amp; Quality Report</title>
  <style>
    body{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;background:#0f172a;color:#f8fafc;padding:30px;margin:0}}
    .container{{max-width:900px;margin:0 auto}}
    h1{{color:#38bdf8;border-bottom:2px solid #334155;padding-bottom:12px}}
    .card{{background:#1e293b;border-radius:8px;padding:20px;margin-bottom:24px;border:1px solid #334155}}
    table{{width:100%;border-collapse:collapse;margin-top:12px}}
    th,td{{text-align:left;padding:10px;border-bottom:1px solid #334155}}
    th{{background:#0f172a;color:#94a3b8;font-size:.85rem;text-transform:uppercase}}
    .footer{{font-size:.8rem;color:#64748b;margin-top:30px}}
  </style>
</head>
<body>
  <div class="container">
    <h1>LinkWatch - Network Audit Report</h1>
    <p>Generated: <strong>{generated_at}</strong></p>
    <div class="card">
      <h2>Diagnostic Verdict</h2>
      <p><b>Status:</b> {status:?}</p>
      <p><b>Finding:</b> {title}</p>
      <p><b>Observation:</b> {obs}</p>
      <p><b>Hypothesis:</b> {hyp}</p>
      <p><b>Confidence:</b> {conf:?}</p>
      <p><b>Recommendation:</b> {rec}</p>
    </div>
    <div class="card">
      <h2>Target Metrics</h2>
      <table><thead><tr>
        <th>Target</th><th>Host</th><th>Probes</th>
        <th>Packet Loss</th><th>Avg RTT</th><th>Min/Max</th>
      </tr></thead><tbody>
"#,
            generated_at = generated_at,
            status = diag.status,
            title = html_escape(&diag.title),
            obs = html_escape(&diag.observation),
            hyp = html_escape(&diag.hypothesis),
            conf = diag.confidence,
            rec = html_escape(&diag.recommendation),
        );

        for s in &stats {
            html.push_str(&format!(
                "<tr><td><b>{}</b></td><td>{}</td><td>{}</td><td>{:.1}%</td><td>{:.1} ms</td><td>{:.1}/{:.1} ms</td></tr>\n",
                html_escape(&s.target_label), html_escape(&s.host), s.total_probes, s.loss_percent,
                s.avg_rtt_ms, s.min_rtt_ms, s.max_rtt_ms
            ));
        }

        html.push_str("</tbody></table></div>\n<div class=\"card\"><h2>Outages</h2><table><thead><tr><th>Start</th><th>Duration</th><th>Affected</th><th>Summary</th></tr></thead><tbody>\n");

        if outages.is_empty() {
            html.push_str("<tr><td colspan=\"4\" style=\"text-align:center;color:#64748b\">No outages recorded.</td></tr>\n");
        } else {
            for o in &outages {
                let start = chrono::DateTime::from_timestamp_millis(o.start_ms)
                    .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| o.start_ms.to_string());
                let dur = o.duration_seconds.map(|d| format!("{d} s")).unwrap_or_else(|| "Ongoing".into());
                html.push_str(&format!("<tr><td>{start}</td><td>{dur}</td><td>{}</td><td>{}</td></tr>\n",
                    html_escape(&o.affected_targets), html_escape(&o.summary)));
            }
        }

        html.push_str("</tbody></table></div>\n<div class=\"footer\">Generated by LinkWatch Desktop Diagnostics Engine - local-first, private.</div>\n</div></body></html>\n");
        Ok(html)
    }
    

    pub fn export_isp_dispute_report(&self) -> Result<String, String> {
        let stats = self.db.get_target_stats().unwrap_or_default();
        let outages = self.db.get_outages(100).unwrap_or_default();
        let diag = self.latest_diagnostic.lock().clone();
        let generated_at = chrono::Utc::now().to_rfc3339();

        let total_probes: i64 = stats.iter().map(|s| s.total_probes).sum();
        let total_success: i64 = stats.iter().map(|s| s.success_count).sum();
        let overall_uptime = if total_probes > 0 {
            (total_success as f64 / total_probes as f64) * 100.0
        } else {
            100.0
        };

        let total_outage_secs: i64 = outages.iter().filter_map(|o| o.duration_seconds).sum();

        let mut html = format!(
            r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Formal ISP Quality of Service & Outage Audit</title>
  <style>
    body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif; background: #ffffff; color: #0f172a; padding: 40px; margin: 0; line-height: 1.5; }}
    .container {{ max-width: 850px; margin: 0 auto; }}
    .header {{ border-bottom: 3px solid #0284c7; padding-bottom: 20px; margin-bottom: 30px; }}
    .title {{ font-size: 24px; font-weight: 800; color: #0f172a; margin: 0; }}
    .subtitle {{ font-size: 14px; color: #64748b; margin-top: 5px; }}
    .notice {{ background: #f8fafc; border: 1px solid #e2e8f0; border-left: 4px solid #0284c7; padding: 15px; border-radius: 6px; margin-bottom: 25px; font-size: 13px; }}
    .metrics-grid {{ display: grid; grid-template-columns: repeat(4, 1fr); gap: 15px; margin-bottom: 30px; }}
    .metric-card {{ background: #f1f5f9; padding: 15px; border-radius: 8px; border: 1px solid #cbd5e1; text-align: center; }}
    .metric-val {{ font-size: 22px; font-weight: 800; color: #0f172a; font-family: monospace; }}
    .metric-lbl {{ font-size: 11px; text-transform: uppercase; color: #64748b; font-weight: 600; margin-top: 4px; }}
    table {{ width: 100%; border-collapse: collapse; margin-top: 15px; font-size: 13px; }}
    th, td {{ text-align: left; padding: 10px 12px; border-bottom: 1px solid #e2e8f0; }}
    th {{ background: #f8fafc; color: #475569; font-size: 11px; text-transform: uppercase; font-weight: 700; }}
    .badge-down {{ background: #fee2e2; color: #991b1b; padding: 2px 6px; border-radius: 4px; font-weight: 700; font-size: 11px; }}
    .footer {{ margin-top: 40px; border-top: 1px solid #e2e8f0; padding-top: 15px; font-size: 11px; color: #94a3b8; text-align: center; }}
  </style>
</head>
<body>
  <div class="container">
    <div class="header">
      <h1 class="title">Internet Service Provider (ISP) Quality of Service Dispute Notice</h1>
      <div class="subtitle">Generated by LinkWatch Autonomous Network Diagnostics &bull; Timestamp: {generated_at}</div>
    </div>

    <div class="notice">
      <strong>Notice to Technical Support / SLA Billing Department:</strong><br>
      This document constitutes empirical evidence of network service interruptions and packet delivery degradation captured continuously via low-overhead ICMP echo telemetry. All measurements isolate local customer-premises router gateway responses from upstream WAN transit hops.
    </div>

    <div class="metrics-grid">
      <div class="metric-card">
        <div class="metric-val">{uptime:.2}%</div>
        <div class="metric-lbl">Measured Uptime</div>
      </div>
      <div class="metric-card">
        <div class="metric-val">{outage_count}</div>
        <div class="metric-lbl">Service Outages</div>
      </div>
      <div class="metric-card">
        <div class="metric-val">{downtime_min}m</div>
        <div class="metric-lbl">Total Downtime</div>
      </div>
      <div class="metric-card">
        <div class="metric-val">{grade} ({score}/100)</div>
        <div class="metric-lbl">Line Quality Grade</div>
      </div>
    </div>

    <h2>Technical Outage & Incident Log</h2>
    <table>
      <thead>
        <tr>
          <th>Incident Start</th>
          <th>Duration</th>
          <th>Affected Scope</th>
          <th>Culprit Isolation</th>
        </tr>
      </thead>
      <tbody>
"#,
            generated_at = generated_at,
            uptime = overall_uptime,
            outage_count = outages.len(),
            downtime_min = (total_outage_secs as f64 / 60.0).ceil() as i64,
            grade = diag.quality_grade,
            score = diag.quality_score,
        );

        if outages.is_empty() {
            html.push_str("<tr><td colspan=\"4\" style=\"text-align:center;color:#64748b;padding:20px;\">No outages recorded during monitoring period.</td></tr>\n");
        } else {
            for o in &outages {
                let start = chrono::DateTime::from_timestamp_millis(o.start_ms)
                    .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_else(|| o.start_ms.to_string());
                let dur = o.duration_seconds.map(|d| format!("{d} sec")).unwrap_or_else(|| "Ongoing".into());
                html.push_str(&format!(
                    "<tr><td><strong>{start}</strong></td><td><span class=\"badge-down\">{dur}</span></td><td>{}</td><td>Upstream ISP / WAN</td></tr>\n",
                    html_escape(&o.affected_targets)
                ));
            }
        }

        html.push_str(r#"
      </tbody>
    </table>

    <div class="footer">
      Generated automatically by LinkWatch Desktop Telemetry Engine &bull; Non-invasive local client verification.
    </div>
  </div>
</body>
</html>
"#);

        Ok(html)
    }// --- Retention & DB stats ------------------------------------------------

    pub fn get_db_stats(&self) -> crate::storage::db::DbStats {
        self.retention.get_db_stats()
    }

    pub fn get_retention_settings(&self) -> crate::storage::db::RetentionSettings {
        self.db.load_retention_settings()
    }

    pub fn save_retention_settings(&self, settings: crate::storage::db::RetentionSettings) -> Result<(), String> {
        self.db.save_retention_settings(&settings).map_err(|e| e.to_string())
    }

    pub fn run_cleanup_now(&self) -> crate::storage::db::DbStats {
        self.retention.run_now()
    }

    pub fn get_target_timeseries(
        &self,
        target_id: &str,
        start_ms: i64,
        end_ms: i64,
        bucket_size_ms: i64,
    ) -> Result<crate::storage::db::TargetTimeSeriesResult, String> {
        self.db.get_target_timeseries(target_id, start_ms, end_ms, bucket_size_ms).map_err(|e| e.to_string())
    }

    pub fn get_start_minimized(&self) -> bool {
        self.db.get_start_minimized()
    }

    pub fn set_start_minimized(&self, enabled: bool) -> Result<(), String> {
        self.db.set_start_minimized(enabled).map_err(|e| e.to_string())
    }

    // ─── Probe interval ───────────────────────────────────────────────────────

    pub fn get_probe_interval_ms(&self) -> u64 {
        self.probe_interval_ms.load(Ordering::Relaxed)
    }

    /// Set probe interval in ms (clamped 500–30000). Takes effect on next cycle.
    pub fn set_probe_interval_ms(&self, interval_ms: u64) -> Result<(), String> {
        let clamped = interval_ms.clamp(500, 30_000);
        self.probe_interval_ms.store(clamped, Ordering::Relaxed);
        self.db.set_probe_interval_ms(clamped).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{csv_quote, record_cycle_writes, MonitorService, DB_WRITE_FAIL_DEGRADE_THRESHOLD};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    #[test]
    fn csv_quote_doubles_embedded_quotes() {
        assert_eq!(csv_quote("Router \"GW\""), "\"Router \"\"GW\"\"\"");
        assert_eq!(csv_quote("plain"), "\"plain\"");
        assert_eq!(csv_quote(""), "\"\"");
    }

    #[test]
    fn degrade_latches_after_threshold_failed_cycles() {
        let streak = AtomicU64::new(0);
        let degraded = AtomicBool::new(false);
        for _ in 0..DB_WRITE_FAIL_DEGRADE_THRESHOLD - 1 {
            record_cycle_writes(&streak, &degraded, 2);
            assert!(!degraded.load(Ordering::SeqCst), "must not fire early");
        }
        record_cycle_writes(&streak, &degraded, 1);
        assert!(degraded.load(Ordering::SeqCst), "must latch at threshold");
    }

    #[test]
    fn clean_cycle_clears_degrade_state() {
        let streak = AtomicU64::new(0);
        let degraded = AtomicBool::new(false);
        for _ in 0..DB_WRITE_FAIL_DEGRADE_THRESHOLD {
            record_cycle_writes(&streak, &degraded, 1);
        }
        assert!(degraded.load(Ordering::SeqCst));
        record_cycle_writes(&streak, &degraded, 0);
        assert!(!degraded.load(Ordering::SeqCst), "recovery must un-latch");
        assert_eq!(streak.load(Ordering::SeqCst), 0, "streak must reset");
    }

    fn service_in_temp_db(tag: &str) -> (MonitorService, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "linkwatch-exit-test-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("linkwatch.db");
        let db = crate::storage::db::Database::init(&db_path).unwrap();
        (MonitorService::new(db, db_path).unwrap(), dir)
    }

    /// The tray Exit used to be undoable: a Resume click mid-shutdown flipped
    /// is_running back to true, so the worker being joined never ended and the
    /// process never exited. start() must be locked out once shutdown ran.
    #[cfg(windows)]
    #[test]
    fn start_after_shutdown_is_locked_out() {
        let (service, dir) = service_in_temp_db("lockout");
        service.start();
        assert!(service.is_running(), "precondition: worker running");
        service.shutdown();
        assert!(!service.is_running(), "precondition: worker stopped");
        service.start();
        assert!(!service.is_running(), "start() after shutdown() must be inert");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A worker parked in a hung getaddrinfo cannot be joined within the
    /// shutdown wait. stop_within must time out and return false so the
    /// exit path can still proceed to kill the process.
    #[cfg(windows)]
    #[test]
    fn stop_within_times_out_on_hung_worker() {
        let (service, dir) = service_in_temp_db("timeout");
        let hung = std::thread::spawn(|| {
            loop { std::thread::sleep(std::time::Duration::from_millis(50)); }
        });
        *service.worker_handle.lock() = Some(hung);
        service.is_running.store(true, Ordering::SeqCst);
        let started = std::time::Instant::now();
        let joined = service.stop_within(std::time::Duration::from_millis(300));
        assert!(!joined, "an unjoinable worker must report false, not hang");
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "wait is bounded");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pause now runs off the main thread, so a Resume click can land while
    /// stop() is still joining. start() must be inert for that window:
    /// flipping is_running mid-join would resurrect the joining worker and
    /// run two probe loops against one database.
    #[cfg(windows)]
    #[test]
    fn start_is_inert_while_stop_is_joining() {
        let (service, dir) = service_in_temp_db("join-lockout");
        let slow_worker = std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(300));
        });
        *service.worker_handle.lock() = Some(slow_worker);
        service.is_running.store(true, Ordering::SeqCst);

        std::thread::scope(|s| {
            let stopper = s.spawn(|| service.stop());
            for _ in 0..100 {
                if service.joining.load(Ordering::SeqCst) { break; }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(service.joining.load(Ordering::SeqCst), "stop must mark joining while it waits");

            service.start();
            assert!(!service.is_running.load(Ordering::SeqCst),
                "start() mid-join must stay locked out");

            let _ = stopper.join();
            assert!(!service.joining.load(Ordering::SeqCst), "joining clears after the join");
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The live repro for "Awaiting Data forever": the real worker must
    /// persist at least one full cycle within 10 s on a working network.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn live_worker_persists_cycles() {
        let (service, dir) = service_in_temp_db("live-cycle");
        service.start();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let written = service.db.get_recent_history(8).map(|v| v.len()).unwrap_or(0);
            if written > 0 {
                service.stop();
                let _ = std::fs::remove_dir_all(&dir);
                assert!(written >= 4, "expected a full cycle (4 built-ins), got {written}");
                return;
            }
            if std::time::Instant::now() > deadline {
                let snap = service.get_snapshot();
                service.stop();
                let _ = std::fs::remove_dir_all(&dir);
                panic!(
                    "worker persisted nothing in 10 s; ring observations={}",
                    snap.latest_observations.len()
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
}