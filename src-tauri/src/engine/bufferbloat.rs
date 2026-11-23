/// engine/bufferbloat.rs - Active bufferbloat & loaded latency stress test.
///
/// Measures:
///  1. Unloaded baseline ping (ICMP)
///  2. Downlink & Uplink TCP saturation burst (HTTP streams)
///  3. Loaded ping under active throughput saturation
///  4. Delta (+ms), bufferbloat grading (A+ down to F)

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferbloatResult {
    pub baseline_ping_ms: f64,
    pub loaded_ping_ms: f64,
    pub bufferbloat_delta_ms: f64,
    pub grade: String,          // A+, A, B, C, D, F
    pub grade_summary: String,
    pub download_mbps: f64,
    pub tested_at_ms: i64,
}

// Bufferbloat grading standard:
// +0-5ms   => A+ (Flawless QoS / SQM router)
// +6-15ms  => A  (Excellent)
// +16-35ms => B  (Good, minimal gaming impact)
// +36-60ms => C  (Noticeable lag spikes during downloads)
// +61-120ms=> D  (Poor, heavy bufferbloat)
// >120ms   => F  (Severe bufferbloat, router queues collapsing)
fn grade_for_delta(delta_ms: f64) -> (&'static str, &'static str) {
    if delta_ms <= 5.0 {
        ("A+", "Flawless - SQM / Smart Queue active. Zero loaded latency bloat.")
    } else if delta_ms <= 15.0 {
        ("A", "Excellent - Minimal buffer latency under full saturation.")
    } else if delta_ms <= 35.0 {
        ("B", "Good - Slight queueing latency, negligible impact on gaming.")
    } else if delta_ms <= 60.0 {
        ("C", "Fair - Noticeable lag spikes when anyone streams or downloads.")
    } else if delta_ms <= 120.0 {
        ("D", "Poor - Significant bufferbloat. Router buffering delays packets.")
    } else {
        ("F", "Severe - Massive buffer congestion. Router desperately needs SQM/CAKE.")
    }
}

pub fn run_bufferbloat_test() -> Result<BufferbloatResult, String> {
    let pinger = crate::engine::icmp::WinIcmpPinger::new()
        .map_err(|e| format!("ICMP initialization error: {e}"))?;

    let test_target = "1.1.1.1";

    // ── Phase 1: Measure Unloaded Ping Baseline (5 probes) ───────────────
    let mut baseline_pings = Vec::new();
    for _ in 0..5 {
        let obs = pinger.ping("bench", "Cloudflare DNS", test_target, 1000);
        if let Some(us) = obs.rtt_measured_us {
            baseline_pings.push(us as f64 / 1000.0);
        }
        std::thread::sleep(Duration::from_millis(60));
    }

    if baseline_pings.is_empty() {
        return Err("Unable to measure baseline ping - check internet connection.".to_string());
    }

    let baseline_avg = baseline_pings.iter().sum::<f64>() / baseline_pings.len() as f64;

    // ── Phase 2: Start Background Saturation Workers ─────────────────────
    let stop_signal = Arc::new(AtomicBool::new(false));
    let bytes_downloaded = Arc::new(std::sync::atomic::AtomicU64::new(0));

    let mut worker_handles = Vec::new();

    // Spawn 4 concurrent TCP download streams from Cloudflare Speed CDN
    for _ in 0..4 {
        let stop_clone = Arc::clone(&stop_signal);
        let bytes_clone = Arc::clone(&bytes_downloaded);

        let handle = std::thread::spawn(move || {
            // Speed test endpoint from Cloudflare CDN (port 80 plain HTTP for zero TLS overhead)
            // Bounded lookup: getaddrinfo on a stalled resolver would otherwise
            // keep this worker alive and the stop signal never reaped.
            if let Some(addr) = crate::engine::icmp::resolve_ipv4("speed.cloudflare.com", Duration::from_secs(2)) {
                let addr = std::net::SocketAddr::new(addr, 80);
                if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                    // Request a 25MB test payload
                    let req = "GET /__down?bytes=25000000 HTTP/1.1\r\nHost: speed.cloudflare.com\r\nUser-Agent: LinkWatch-Bench\r\nConnection: close\r\n\r\n";
                    if stream.write_all(req.as_bytes()).is_ok() {
                        let mut buf = [0u8; 16384];
                        while !stop_clone.load(Ordering::Relaxed) {
                            match stream.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    bytes_clone.fetch_add(n as u64, Ordering::Relaxed);
                                }
                                Err(_) => break,
                            }
                        }
                    }
                }
            }
        });
        worker_handles.push(handle);
    }

    // Let saturation build for 1 second
    std::thread::sleep(Duration::from_millis(1000));
    let burst_start = Instant::now();
    // Only count bytes moved after the warm-up; traffic accumulated during the
    // 1 s build-up would otherwise be charged to the measured window and
    // inflate download_mbps.
    let bytes_at_burst = bytes_downloaded.load(Ordering::Relaxed);

    // ── Phase 3: Probe Latency Under Load (6 probes) ─────────────────────
    let mut loaded_pings = Vec::new();
    for _ in 0..6 {
        let obs = pinger.ping("bench", "Cloudflare DNS", test_target, 1500);
        if let Some(us) = obs.rtt_measured_us {
            loaded_pings.push(us as f64 / 1000.0);
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    // Stop workers
    stop_signal.store(true, Ordering::Relaxed);
    for h in worker_handles {
        let _ = h.join();
    }

    let elapsed_sec = burst_start.elapsed().as_secs_f64().max(0.5);
    let total_bytes = bytes_downloaded.load(Ordering::Relaxed).saturating_sub(bytes_at_burst);
    let download_mbps = ((total_bytes as f64 * 8.0) / 1_000_000.0) / elapsed_sec;

    let loaded_avg = if !loaded_pings.is_empty() {
        loaded_pings.iter().sum::<f64>() / loaded_pings.len() as f64
    } else {
        // Every probe was lost while the link was saturated: that itself is
        // severe congestion, so it must land in the F band (>120 ms delta),
        // not the old +80 which silently graded it D.
        baseline_avg + 200.0
    };

    let delta_ms = (loaded_avg - baseline_avg).max(0.0);

    let (grade, summary) = grade_for_delta(delta_ms);

    Ok(BufferbloatResult {
        baseline_ping_ms: (baseline_avg * 10.0).round() / 10.0,
        loaded_ping_ms: (loaded_avg * 10.0).round() / 10.0,
        bufferbloat_delta_ms: (delta_ms * 10.0).round() / 10.0,
        grade: grade.to_string(),
        grade_summary: summary.to_string(),
        download_mbps: (download_mbps * 10.0).round() / 10.0,
        tested_at_ms: chrono::Utc::now().timestamp_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::grade_for_delta;

    #[test]
    fn grade_band_boundaries() {
        assert_eq!(grade_for_delta(0.0).0, "A+");
        assert_eq!(grade_for_delta(5.0).0, "A+");
        assert_eq!(grade_for_delta(5.1).0, "A");
        assert_eq!(grade_for_delta(120.0).0, "D");
        assert_eq!(grade_for_delta(120.1).0, "F");
        // The all-probes-lost fallback (+200 ms) must grade F, the old +80
        // silently graded total congestion as D.
        assert_eq!(grade_for_delta(200.0).0, "F");
    }
}
