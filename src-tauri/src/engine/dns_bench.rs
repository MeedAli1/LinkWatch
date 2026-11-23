use serde::{Deserialize, Serialize};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsResolverResult {
    pub id: String,
    pub name: String,
    pub server_ip: String,
    pub avg_latency_ms: f64,
    pub min_latency_ms: f64,
    pub max_latency_ms: f64,
    pub success_count: u32,
    pub query_count: u32,
    pub is_fastest: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsBenchmarkResult {
    pub resolvers: Vec<DnsResolverResult>,
    pub fastest_resolver_name: String,
    pub recommendation: String,
    pub tested_at_ms: i64,
}

/// Creates a standard raw DNS query packet for an A record.
/// Flags: 0x0100 (standard query, recursion desired), 1 Question, 0 Answers.
/// The caller supplies a per-query transaction ID so replies can be verified.
fn build_dns_query(domain: &str, txid: u16) -> Vec<u8> {
    let mut packet = Vec::with_capacity(64);
    // Header
    packet.extend_from_slice(&txid.to_be_bytes()); // ID
    packet.extend_from_slice(&[0x01, 0x00]); // Flags: Standard query, recursion desired
    packet.extend_from_slice(&[0x00, 0x01]); // Questions: 1
    packet.extend_from_slice(&[0x00, 0x00]); // Answer RRs: 0
    packet.extend_from_slice(&[0x00, 0x00]); // Authority RRs: 0
    packet.extend_from_slice(&[0x00, 0x00]); // Additional RRs: 0

    // Question Name: sequence of labels (length byte followed by label bytes)
    for part in domain.split('.') {
        if !part.is_empty() {
            packet.push(part.len() as u8);
            packet.extend_from_slice(part.as_bytes());
        }
    }
    packet.push(0x00); // End of name (null label)

    // Question Type & Class
    packet.extend_from_slice(&[0x00, 0x01]); // Type: A (IPv4 address)
    packet.extend_from_slice(&[0x00, 0x01]); // Class: IN (Internet)

    packet
}

/// Queries a specific DNS server over UDP and measures round-trip latency in milliseconds.
/// A reply counts only when its transaction ID matches this query and it
/// arrives from port 53; stray or spoofed packets are ignored until the real
/// answer arrives or the timeout expires.
pub fn measure_dns_query(server_ip: &str, domain: &str, timeout: Duration) -> Option<f64> {
    let addr: SocketAddr = format!("{}:53", server_ip).parse().ok()?;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.set_read_timeout(Some(timeout)).ok()?;
    socket.set_write_timeout(Some(timeout)).ok()?;

    // Unique per query so a late/spoofed answer to an earlier one can't pass.
    let txid = (chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u16)
        ^ (std::process::id() as u16);
    let query = build_dns_query(domain, txid);
    let start = Instant::now();

    if socket.send_to(&query, addr).is_err() {
        return None;
    }

    let mut buf = [0u8; 512];
    loop {
        match socket.recv_from(&mut buf) {
            Ok((amt, from)) if amt > 12 && from.port() == 53 && buf[0..2] == txid.to_be_bytes() => {
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                return Some(elapsed);
            }
            Ok(_) => continue,  // not our reply; keep waiting within the read timeout
            Err(_) => return None, // timeout or socket error
        }
    }
}

/// Runs a comprehensive DNS benchmark testing multiple domains across top public resolvers.
pub fn run_dns_benchmark() -> DnsBenchmarkResult {
    let test_domains = ["google.com", "cloudflare.com", "microsoft.com", "wikipedia.org"];

    let target_resolvers = [
        ("cloudflare", "Cloudflare DNS", "1.1.1.1"),
        ("google", "Google Public DNS", "8.8.8.8"),
        ("quad9", "Quad9 Secure", "9.9.9.9"),
        ("opendns", "Cisco OpenDNS", "208.67.222.222"),
        ("adguard", "AdGuard DNS", "94.140.14.14"),
    ];

    let mut results = Vec::new();
    let timeout = Duration::from_millis(1500);

    for (id, name, ip) in target_resolvers {
        let mut latencies: Vec<f64> = Vec::new();
        let mut success_count = 0u32;
        let query_count = test_domains.len() as u32;

        for domain in test_domains {
            if let Some(rtt) = measure_dns_query(ip, domain, timeout) {
                latencies.push(rtt);
                success_count += 1;
            }
        }

        let avg = if !latencies.is_empty() {
            latencies.iter().sum::<f64>() / latencies.len() as f64
        } else {
            999.0
        };

        let min = latencies.iter().copied().fold(f64::INFINITY, f64::min);
        let max = latencies.iter().copied().fold(0.0f64, f64::max);

        results.push(DnsResolverResult {
            id: id.to_string(),
            name: name.to_string(),
            server_ip: ip.to_string(),
            avg_latency_ms: avg,
            min_latency_ms: if min.is_infinite() { 0.0 } else { min },
            max_latency_ms: max,
            success_count,
            query_count,
            is_fastest: false,
        });
    }

    // Find the fastest responder with at least 1 success
    let mut fastest_idx = 0;
    let mut fastest_avg = f64::INFINITY;

    for (idx, r) in results.iter().enumerate() {
        if r.success_count > 0 && r.avg_latency_ms < fastest_avg {
            fastest_avg = r.avg_latency_ms;
            fastest_idx = idx;
        }
    }

    if !results.is_empty() && fastest_avg < 900.0 {
        results[fastest_idx].is_fastest = true;
    }

    let fastest_name = results
        .iter()
        .find(|r| r.is_fastest)
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "Cloudflare DNS".to_string());

    let recommendation = format!(
        "{} delivered the lowest lookup latency ({:.1}ms average). Switching to it can noticeably improve browsing responsiveness.",
        fastest_name, fastest_avg
    );

    DnsBenchmarkResult {
        resolvers: results,
        fastest_resolver_name: fastest_name,
        recommendation,
        tested_at_ms: chrono::Utc::now().timestamp_millis(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_dns_query_packet() {
        let packet = build_dns_query("example.com", 0x1234);
        // Header is 12 bytes, "example" is 1+7=8 bytes, "com" is 1+3=4 bytes, null byte is 1 byte, type/class is 4 bytes = 29 bytes
        assert_eq!(packet.len(), 29);
        assert_eq!(packet[0], 0x12);
        assert_eq!(packet[1], 0x34);
        assert_eq!(packet[12], 7); // length of "example"
    }

    #[test]
    fn query_carries_supplied_txid() {
        let packet = build_dns_query("a.test", 0xBEEF);
        assert_eq!(&packet[0..2], &[0xBE, 0xEF]);
        let other = build_dns_query("a.test", 0x0001);
        assert_ne!(&packet[0..2], &other[0..2]);
    }

    /// Live check that real server answers survive txid verification on this
    /// network: cargo test --lib -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_dns_answers_pass_txid_check() {
        let r = measure_dns_query("1.1.1.1", "google.com", Duration::from_millis(1500));
        println!("1.1.1.1 rtt = {r:?} ms");
        assert!(r.is_some(), "live server answer must pass txid/port verification");
    }
}
