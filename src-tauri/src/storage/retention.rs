use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::storage::db::{Database, DbStats, RetentionSettings};

const BATCH: usize = 1000;
/// How often the background thread wakes to run a pruning cycle (5 minutes).
const CYCLE_SECS: u64 = 300;
/// Sleep granularity for interruptible waits (10 seconds).
const TICK_SECS: u64 = 10;

pub struct RetentionService {
    db: Arc<Database>,
    is_running: Arc<AtomicBool>,
    db_path: PathBuf,
    worker_handle: parking_lot::Mutex<Option<JoinHandle<()>>>,
}

impl RetentionService {
    pub fn new(db: Arc<Database>, db_path: PathBuf) -> Self {
        RetentionService {
            db,
            is_running: Arc::new(AtomicBool::new(false)),
            db_path,
            worker_handle: parking_lot::Mutex::new(None),
        }
    }

    /// Start the background pruning thread.
    pub fn start(&self) {
        if self.is_running.swap(true, Ordering::SeqCst) {
            return; // already running
        }

        let db = Arc::clone(&self.db);
        let is_running = Arc::clone(&self.is_running);

        let handle = thread::spawn(move || {
            let ticks_per_cycle = (CYCLE_SECS / TICK_SECS) as u32;
            let mut tick_count = ticks_per_cycle; // run immediately on first cycle

            while is_running.load(Ordering::SeqCst) {
                // Sleep with short 100ms granularity so shutdown joins immediately
                for _ in 0..100 {
                    if !is_running.load(Ordering::SeqCst) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }

                tick_count += 1;
                if tick_count < ticks_per_cycle {
                    continue;
                }
                tick_count = 0;

                // Load fresh settings each cycle (user may have changed them)
                let settings = db.load_retention_settings();
                if settings.pruning_enabled {
                    run_pruning_cycle(&db, &settings);
                }
            }
        });

        *self.worker_handle.lock() = Some(handle);
    }

    /// Stop the background thread gracefully.
    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
        if let Some(h) = self.worker_handle.lock().take() {
            let _ = h.join();
        }
    }

    /// Run one pruning cycle synchronously (called from "Run Cleanup Now" button).
    pub fn run_now(&self) -> DbStats {
        let settings = self.db.load_retention_settings();
        if settings.pruning_enabled {
            run_pruning_cycle(&self.db, &settings);
        }
        let wal_path = {
            let mut p = self.db_path.clone();
            let fname = p.file_name().map(|n| {
                let mut s = n.to_os_string();
                s.push("-wal");
                s
            });
            if let Some(f) = fname {
                p.set_file_name(f);
                Some(p)
            } else {
                None
            }
        };
        self.db.get_db_stats(wal_path.as_deref())
    }

    /// Return current DB stats (for the Settings panel).
    pub fn get_db_stats(&self) -> DbStats {
        let wal_path = {
            let mut p = self.db_path.clone();
            let fname = p.file_name().map(|n| {
                let mut s = n.to_os_string();
                s.push("-wal");
                s
            });
            if let Some(f) = fname {
                p.set_file_name(f);
                Some(p)
            } else {
                None
            }
        };
        self.db.get_db_stats(wal_path.as_deref())
    }
}

/// Core pruning logic shared by background loop and "run now" path.
fn run_pruning_cycle(db: &Database, settings: &RetentionSettings) {
    // Age-based pruning
    if settings.max_age_days > 0 {
        let cutoff_ms = chrono::Utc::now().timestamp_millis()
            - (settings.max_age_days as i64 * 24 * 3600 * 1000);
        let _ = db.prune_by_age(cutoff_ms, BATCH);
        let _ = db.prune_outages(cutoff_ms);
    }

    // Count-based pruning
    if settings.max_records > 0 {
        let _ = db.prune_by_count(settings.max_records as usize, BATCH);
    }

    // Passive WAL checkpoint – does not block readers or writers
    let _ = db.checkpoint_passive();

    // Record last cleanup timestamp
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut updated = settings.clone();
    updated.last_cleanup_ms = Some(now_ms);
    let _ = db.save_retention_settings(&updated);
}
