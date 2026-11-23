use std::collections::HashMap;
use std::mem;
use std::net::{IpAddr, ToSocketAddrs};
use std::ptr;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use std::sync::Arc;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, IP_OPTION_INFORMATION,
};
use windows_sys::Win32::Networking::WinSock::{
    WSACleanup, WSAGetLastError, WSAStartup, WSADATA,
};

/// Caches DNS lookups to avoid a syscall per probe cycle. A failure is cached
/// for `NEGATIVE_TTL` and a success for `TTL`; caching the failure is what
/// keeps a dead hostname from spending every cycle's bounded resolve wait.
#[derive(Default)]
pub struct DnsCache {
    inner: HashMap<String, (CachedLookup, Instant)>,
}

#[derive(Clone, Copy)]
enum CachedLookup {
    Ok(IpAddr),
    Failed,
}

impl DnsCache {
    const TTL: Duration = Duration::from_secs(30);
    const NEGATIVE_TTL: Duration = Duration::from_secs(5);

    fn ttl_for(lookup: &CachedLookup) -> Duration {
        match lookup {
            CachedLookup::Ok(_) => Self::TTL,
            CachedLookup::Failed => Self::NEGATIVE_TTL,
        }
    }

    /// Returns the cached verdict for a fresh-enough entry: `Some(Some(ip))`
    /// is resolved, `Some(None)` is a known failure, `None` means "resolve
    /// again". Resolution itself must happen while the lock is released (see
    /// `ping`), so only these accessors touch the map.
    pub fn cached(&self, host: &str) -> Option<Option<IpAddr>> {
        let (lookup, ts) = self.inner.get(host)?;
        if ts.elapsed() < Self::ttl_for(lookup) {
            Some(match lookup {
                CachedLookup::Ok(ip) => Some(*ip),
                CachedLookup::Failed => None,
            })
        } else {
            None
        }
    }

    pub fn insert(&mut self, host: &str, ip: Option<IpAddr>) {
        let lookup = match ip {
            Some(ip) => CachedLookup::Ok(ip),
            None => CachedLookup::Failed,
        };
        self.inner.insert(host.to_string(), (lookup, Instant::now()));
    }

    /// Invalidate a single host (called on probe failure so we re-resolve next cycle).
    pub fn invalidate(&mut self, host: &str) {
        self.inner.remove(host);
    }
}

/// Resolves `host` to its first IPv4 address, giving up after `wait`.
///
/// `getaddrinfo` has no timeout of its own: a stalled resolver can park the
/// caller indefinitely, which is what froze the probe loop and then the UI on
/// Pause. The lookup therefore runs on its own thread and the wait is bounded.
/// On timeout the thread stays parked detached - it holds only a `String` and
/// a channel sender, and the negative cache caps the leak at one thread per
/// host per `DnsCache::NEGATIVE_TTL`.
pub fn resolve_ipv4(host: &str, wait: Duration) -> Option<IpAddr> {
    let keyed = format!("{host}:0");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let ip = keyed
            .to_socket_addrs()
            .ok()
            .and_then(|addrs| addrs.into_iter().find(|a| a.is_ipv4()).map(|a| a.ip()));
        let _ = tx.send(ip);
    });
    match rx.recv_timeout(wait) {
        Ok(ip) => ip,
        Err(_) => None,
    }
}

#[repr(C)]
pub struct IcmpEchoReply {
    pub address: u32,
    pub status: u32,
    pub round_trip_time: u32,
    pub data_size: u16,
    pub reserved: u16,
    pub data: *mut u8,
    pub options: IP_OPTION_INFORMATION,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProbeObservation {
    pub target_id: String,
    pub target_label: String,
    pub host: String,
    pub resolved_ip: Option<String>,
    pub timestamp_ms: i64,
    pub rtt_api_ms: Option<u32>,
    pub rtt_measured_us: Option<u64>,
    pub success: bool,
    pub status_code: u32,
    pub error_msg: Option<String>,
}

pub struct WinIcmpPinger {
    handle: HANDLE,
    pub dns_cache: Arc<Mutex<DnsCache>>,
}

/// How long a probe thread waits for the bounded resolver before calling the
/// target unresolvable. Kept under the probe cycle interval so one dead name
/// cannot starve the rest of the cycle.
const DNS_RESOLVE_WAIT: Duration = Duration::from_millis(1000);

fn dns_failure(
    target_id: &str,
    target_label: &str,
    host: &str,
    now_ms: i64,
) -> ProbeObservation {
    ProbeObservation {
        target_id: target_id.to_string(),
        target_label: target_label.to_string(),
        host: host.to_string(),
        resolved_ip: None,
        timestamp_ms: now_ms,
        rtt_api_ms: None,
        rtt_measured_us: None,
        success: false,
        status_code: 0,
        error_msg: Some("DNS resolve failed".to_string()),
    }
}

unsafe impl Send for WinIcmpPinger {}
unsafe impl Sync for WinIcmpPinger {}

impl WinIcmpPinger {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let mut wsa_data: WSADATA = mem::zeroed();
            let res = WSAStartup(0x0202, &mut wsa_data);
            if res != 0 {
                return Err(format!("WSAStartup failed: {res}"));
            }

            let handle = IcmpCreateFile();
            if handle.is_null() || handle == -1isize as HANDLE {
                return Err("IcmpCreateFile failed to open ICMP handle".to_string());
            }
            Ok(WinIcmpPinger {
                handle,
                dns_cache: Arc::new(Mutex::new(DnsCache::default())),
            })
        }
    }

    pub fn ping(&self, target_id: &str, target_label: &str, host: &str, timeout_ms: u32) -> ProbeObservation {
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Bind the cached verdict before touching the lock again. In Rust
        // 2021 a match scrutinee temporary - here the MutexGuard - lives
        // through the arms, and parking_lot is not reentrant: re-locking
        // inside an arm deadlocks the thread on the first cache miss. That
        // deadlock is what starved every probe cycle, emptied the
        // dashboard, and made Pause's join hang the UI forever.
        let cached = self.dns_cache.lock().cached(host);
        let ip = match cached {
            Some(Some(ip)) => ip,
            Some(None) => return dns_failure(target_id, target_label, host, now_ms),
            None => {
                let fresh = resolve_ipv4(host, DNS_RESOLVE_WAIT);
                self.dns_cache.lock().insert(host, fresh);
                match fresh {
                    Some(ip) => ip,
                    None => return dns_failure(target_id, target_label, host, now_ms),
                }
            }
        };

        let ipv4_octets = match ip {
            IpAddr::V4(v4) => v4.octets(),
            _ => unreachable!(),
        };
        let dest_addr = u32::from_ne_bytes(ipv4_octets);

        let send_data = b"LinkWatch-Probe-Data";
        let reply_size = mem::size_of::<IcmpEchoReply>() + send_data.len() + 8;
        let mut reply_buffer: Vec<u8> = vec![0u8; reply_size];

        let start = Instant::now();
        let replies_count = unsafe {
            IcmpSendEcho(
                self.handle,
                dest_addr,
                send_data.as_ptr() as *const _,
                send_data.len() as u16,
                ptr::null(),
                reply_buffer.as_mut_ptr() as *mut _,
                reply_size as u32,
                timeout_ms,
            )
        };
        let elapsed_us = start.elapsed().as_micros() as u64;

        if replies_count > 0 {
            let reply = unsafe { &*(reply_buffer.as_ptr() as *const IcmpEchoReply) };
            let success = reply.status == 0;
            if !success {
                // Invalidate cache on failure so next cycle re-resolves
                self.dns_cache.lock().invalidate(host);
            }
            ProbeObservation {
                target_id: target_id.to_string(),
                target_label: target_label.to_string(),
                host: host.to_string(),
                resolved_ip: Some(ip.to_string()),
                timestamp_ms: now_ms,
                rtt_api_ms: Some(reply.round_trip_time),
                rtt_measured_us: Some(elapsed_us),
                success,
                status_code: reply.status,
                error_msg: if success {
                    None
                } else {
                    Some(format!("IP status {}", reply.status))
                },
            }
        } else {
            let error = unsafe { WSAGetLastError() };
            // Timeout/error - invalidate so we re-resolve next cycle
            self.dns_cache.lock().invalidate(host);
            ProbeObservation {
                target_id: target_id.to_string(),
                target_label: target_label.to_string(),
                host: host.to_string(),
                resolved_ip: Some(ip.to_string()),
                timestamp_ms: now_ms,
                rtt_api_ms: None,
                rtt_measured_us: Some(elapsed_us),
                success: false,
                status_code: error as u32,
                error_msg: Some(format!("Timeout or WSA error {error}")),
            }
        }
    }
}

impl Drop for WinIcmpPinger {
    fn drop(&mut self) {
        unsafe {
            if !self.handle.is_null() && self.handle != -1isize as HANDLE {
                IcmpCloseHandle(self.handle);
            }
            WSACleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last))
    }

    #[test]
    fn cached_hit_returns_the_stored_address() {
        let mut cache = DnsCache::default();
        cache.insert("a.test", Some(ip(1)));
        assert_eq!(cache.cached("a.test"), Some(Some(ip(1))));
    }

    #[test]
    fn cached_failure_is_reported_as_known_unresolved() {
        let mut cache = DnsCache::default();
        cache.insert("dead.test", None);
        assert_eq!(cache.cached("dead.test"), Some(None));
    }

    #[test]
    fn unknown_host_is_a_miss() {
        let cache = DnsCache::default();
        assert_eq!(cache.cached("never.test"), None);
    }

    #[test]
    fn invalidate_forces_the_next_cycle_to_re_resolve() {
        let mut cache = DnsCache::default();
        cache.insert("flap.test", Some(ip(9)));
        cache.invalidate("flap.test");
        assert_eq!(cache.cached("flap.test"), None);
    }

    #[test]
    fn failures_expire_faster_than_successes() {
        // A negative entry must be retried sooner than a working address,
        // otherwise a transient resolver outage sticks for the full TTL.
        assert!(DnsCache::ttl_for(&CachedLookup::Failed) < DnsCache::ttl_for(&CachedLookup::Ok(ip(1))));
    }

    #[test]
    fn numeric_host_resolves_within_the_bound() {
        let got = resolve_ipv4("203.0.113.7", Duration::from_secs(2));
        assert_eq!(got, Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))));
    }

    /// Regression for the total-monitoring freeze: the first cache miss used
    /// to re-lock dns_cache inside the match arm while the scrutinee guard
    /// was still alive (parking_lot is not reentrant), deadlocking every
    /// probe thread. Loopback needs no external network; the assertion is
    /// that ping returns at all.
    #[cfg(windows)]
    #[test]
    fn first_cache_miss_does_not_deadlock() {
        let p = WinIcmpPinger::new().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let o = p.ping("t", "T", "127.0.0.1", 300);
            let _ = tx.send(o);
        });
        let obs = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("ping must return, not deadlock on the cache lock");
        assert_eq!(obs.target_id, "t");
    }

    /// Live repro part 1: one ping on a fresh handle must return within its
    /// timeout even when the network misbehaves.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn single_ping_is_bounded() {
        let p = WinIcmpPinger::new().unwrap();
        let t = Instant::now();
        let o = p.ping("t", "T", "1.1.1.1", 1200);
        println!("single ping: ok={} elapsed={:?}", o.success, t.elapsed());
        assert!(t.elapsed() < Duration::from_secs(5), "IcmpSendEcho outran its timeout");
    }

    /// Live repro part 2: the probe loop fans 4+ threads onto ONE shared
    /// ICMP handle. If concurrent IcmpSendEcho calls starve each other, the
    /// scope never closes and the worker becomes unjoinable.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn concurrent_pings_on_one_handle_are_bounded() {
        let p = Arc::new(WinIcmpPinger::new().unwrap());
        let t = Instant::now();
        std::thread::scope(|s| {
            for i in 0..4 {
                let p = Arc::clone(&p);
                s.spawn(move || {
                    let o = p.ping(&format!("t{i}"), "T", "1.1.1.1", 1200);
                    println!("thread {i}: ok={} elapsed={:?}", o.success, t.elapsed());
                });
            }
        });
        println!("4 concurrent pings: {:?}", t.elapsed());
        assert!(t.elapsed() < Duration::from_secs(10), "shared-handle concurrency stalls");
    }
}
