use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputData {
    pub download_kbps: f64,
    pub upload_kbps: f64,
    pub download_mbps: f64,
    pub upload_mbps: f64,
    pub interface_name: String,
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
}

/// One (rx, tx, timestamp) baseline. The main window and the overlay poll at
/// different rates, so all three fields must be read and replaced as a unit;
/// separate atomics let one caller steal the other's timestamp and corrupt the
/// delta.
#[derive(Clone, Copy)]
struct PrevSample {
    rx: u64,
    tx: u64,
    t_ms: u64,
}

static PREV: Mutex<PrevSample> = Mutex::new(PrevSample { rx: 0, tx: 0, t_ms: 0 });

pub struct ThroughputMonitor;

/// Locale-proof byte counters via the IP Helper MIB: sums the 64-bit octet
/// counters of every operational adapter and names the busiest one. Replaces
/// `netstat -e` parsing, whose "Bytes" row label is translated on localized
/// Windows, which silently zeroed all throughput.
#[cfg(target_os = "windows")]
fn adapter_byte_counters() -> Option<(u64, u64, String)> {
    use windows_sys::Win32::NetworkManagement::{
        IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2},
        Ndis::IfOperStatusUp,
    };

    // other(1), tunnel(21), ISDN(22), PPP(23), software loopback(24)
    const EXCLUDED_IF_TYPES: [u32; 5] = [1, 21, 22, 23, 24];
    // Empirically on this machine, the InterfaceAndOperStatusFlags
    // FilterInterface bit (1) is set on every WFP/QoS/Hyper-V lightweight
    // filter miniport — these duplicate their parent adapter's octet counters
    // — and clear on all base adapters.
    const FLAG_FILTER_INTERFACE: u8 = 0b0000_0010;

    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table) != 0 || table.is_null() {
            return None;
        }
        let header = &*table;
        let rows = std::slice::from_raw_parts(header.Table.as_ptr(), header.NumEntries as usize);
        let (mut rx, mut tx) = (0u64, 0u64);
        let mut busiest = (0u64, String::new());
        for row in rows {
            if row.OperStatus != IfOperStatusUp
                || EXCLUDED_IF_TYPES.contains(&row.Type)
                || row.InterfaceAndOperStatusFlags._bitfield & FLAG_FILTER_INTERFACE != 0
            {
                continue;
            }
            rx = rx.saturating_add(row.InOctets);
            tx = tx.saturating_add(row.OutOctets);
            let volume = row.InOctets.saturating_add(row.OutOctets);
            if volume > busiest.0 {
                let name = String::from_utf16_lossy(&row.Alias)
                    .trim_end_matches('\0')
                    .to_string();
                busiest = (volume, name);
            }
        }
        FreeMibTable(table as *const _);
        Some((rx, tx, busiest.1))
    }
}

/// Debug aid for `live_machine_check`: one line per interface row.
#[cfg(target_os = "windows")]
pub fn if_table_debug_lines() -> Vec<String> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, MIB_IF_TABLE2,
    };
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table) != 0 || table.is_null() {
            return Vec::new();
        }
        let header = &*table;
        let rows = std::slice::from_raw_parts(header.Table.as_ptr(), header.NumEntries as usize);
        let out = rows.iter().map(|r| {
            format!(
                "type={} oper={} flags={:08b} rx={} | {}",
                r.Type,
                r.OperStatus,
                r.InterfaceAndOperStatusFlags._bitfield,
                r.InOctets,
                String::from_utf16_lossy(&r.Alias).trim_end_matches('\0')
            )
        }).collect();
        FreeMibTable(table as *const _);
        out
    }
}

/// Rate for the interval between two samples, in kbps. Returns None when the
/// interval is unusable (first sample, or clock/counter going backwards).
fn rates_kbps(prev: &PrevSample, cur: &PrevSample) -> Option<(f64, f64)> {
    if prev.t_ms == 0 || cur.t_ms <= prev.t_ms {
        return None;
    }
    let delta_sec = (cur.t_ms - prev.t_ms) as f64 / 1000.0;
    let rx = cur.rx.saturating_sub(prev.rx);
    let tx = cur.tx.saturating_sub(prev.tx);
    Some(((rx as f64 / 1024.0) / delta_sec, (tx as f64 / 1024.0) / delta_sec))
}

impl ThroughputMonitor {
    /// Reads network adapter byte counters from the Windows interface MIB.
    pub fn get_throughput() -> ThroughputData {
        #[cfg(target_os = "windows")]
        let (total_rx, total_tx, name) =
            adapter_byte_counters().unwrap_or((0, 0, String::new()));
        #[cfg(not(target_os = "windows"))]
        let (total_rx, total_tx, name) = (0u64, 0u64, String::new());

        let interface_name = if name.is_empty() {
            "Active Interface".to_string()
        } else {
            name
        };

        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let prev = {
            let mut g = PREV.lock();
            let p = *g;
            *g = PrevSample { rx: total_rx, tx: total_tx, t_ms: now_ms };
            p
        };

        let (download_kbps, upload_kbps) = match rates_kbps(
            &prev,
            &PrevSample { rx: total_rx, tx: total_tx, t_ms: now_ms },
        ) {
            Some(r) => r,
            None => (0.0, 0.0),
        };

        ThroughputData {
            download_kbps,
            upload_kbps,
            download_mbps: download_kbps / 1024.0,
            upload_mbps: upload_kbps / 1024.0,
            interface_name,
            total_rx_bytes: total_rx,
            total_tx_bytes: total_tx,
        }
    }
}

impl Default for ThroughputData {
    fn default() -> Self {
        ThroughputData {
            download_kbps: 0.0,
            upload_kbps: 0.0,
            download_mbps: 0.0,
            upload_mbps: 0.0,
            interface_name: "Primary".to_string(),
            total_rx_bytes: 0,
            total_tx_bytes: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(rx: u64, tx: u64, t_ms: u64) -> PrevSample {
        PrevSample { rx, tx, t_ms }
    }

    #[test]
    fn first_sample_has_no_rate() {
        assert_eq!(rates_kbps(&sample(0, 0, 0), &sample(100, 100, 5000)), None);
    }

    #[test]
    fn backwards_clock_is_rejected() {
        assert_eq!(rates_kbps(&sample(1000, 0, 5000), &sample(1100, 0, 4000)), None);
    }

    #[test]
    fn known_interval_computes_known_rate() {
        // 10240 bytes over 10 s = 1 KiB/s down, 0 up
        let (dl, ul) = rates_kbps(&sample(1_000_000, 500_000, 10_000), &sample(1_010_240, 500_000, 20_000)).unwrap();
        assert!((dl - 1.0).abs() < 1e-9);
        assert_eq!(ul, 0.0);
    }

    #[test]
    fn interleaved_pollers_never_split_one_interval() {
        // Overlay (1 s) then main (2 s) against the same shared baseline:
        // each interval must account exactly for the bytes moved in it.
        let a = sample(0, 0, 1_000);
        let b = sample(1_024, 0, 2_000); // +1 KiB over 1 s
        let c = sample(3_072, 0, 4_000); // +2 KiB over 2 s
        let (ab, _) = rates_kbps(&a, &b).unwrap();
        let (bc, _) = rates_kbps(&b, &c).unwrap();
        assert_eq!(ab, 1.0);
        assert_eq!(bc, 1.0);
    }

    #[test]
    fn counter_reset_saturates_to_zero_not_wrap() {
        let (dl, _) = rates_kbps(&sample(u64::MAX, 0, 1_000), &sample(500, 0, 2_000)).unwrap();
        assert_eq!(dl, 0.0);
    }
}
