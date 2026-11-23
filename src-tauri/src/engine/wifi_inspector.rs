use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiDetails {
    pub is_connected: bool,
    pub ssid: String,
    pub bssid: String,
    pub signal_percent: u32,
    pub channel: u32,
    pub band: String,
    pub link_speed_mbps: u32,
    pub interface_name: String,
    pub security: String,
    pub health_verdict: String,
}

impl Default for WifiDetails {
    fn default() -> Self {
        WifiDetails {
            is_connected: false,
            ssid: "Not Connected".to_string(),
            bssid: "".to_string(),
            signal_percent: 0,
            channel: 0,
            band: "Unknown".to_string(),
            link_speed_mbps: 0,
            interface_name: "Wi-Fi".to_string(),
            security: "Unknown".to_string(),
            health_verdict: "No active Wi-Fi interface detected or Ethernet in use.".to_string(),
        }
    }
}

/// A nearby Wi-Fi network detected by scanning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiNeighbor {
    pub ssid: String,
    pub bssid: String,
    pub signal_percent: u32,
    pub channel: u32,
    pub band: String,
    pub security: String,
    pub is_current: bool,
}

/// Channel congestion summary for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelCongestion {
    pub channel: u32,
    pub band: String,
    pub network_count: u32,
    pub strongest_signal: u32,
    pub is_your_channel: bool,
}

/// Full WiFi environment scan result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiEnvironmentScan {
    pub your_network: WifiDetails,
    pub neighbors: Vec<WifiNeighbor>,
    pub channel_congestion: Vec<ChannelCongestion>,
    pub interference_verdict: String,
}

/// Splits one netsh `Label : Value` line into a lowercase label and trimmed value.
fn split_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let (key, val) = trimmed.split_once(':')?;
    Some((key.trim().to_lowercase(), val.trim().to_string()))
}

/// Matches signal values like "85%" regardless of the (translated) label.
fn parse_pct(val: &str) -> Option<u32> {
    val.strip_suffix('%')?.trim().parse().ok()
}

/// Wi-Fi channels are the only standalone small integers in netsh output
/// (upper bound 230 covers 2.4/5/6 GHz channel numbering).
fn parse_channel(val: &str) -> Option<u32> {
    let n: u32 = val.parse().ok()?;
    (n >= 1 && n <= 230).then_some(n)
}

/// Rates look like "72.2 Mbps" (en), "57,8 MBit/s" (de). On fr-FR netsh the
/// unit sits in the *label* ("Réception (Mbits/s) : 459"), so the caller also
/// checks the key.
fn parse_rate_mbps(val: &str) -> Option<f64> {
    let first = val.split_whitespace().next()?;
    if val.to_lowercase().contains("mbps") || val.to_lowercase().contains("mbit") {
        return first.replace(',', ".").parse::<f64>().ok();
    }
    None
}

fn rate_value(key: &str, val: &str) -> Option<f64> {
    if let Some(r) = parse_rate_mbps(val) {
        return Some(r);
    }
    if key.contains("mbit") || key.contains("mbps") {
        return val.split_whitespace().next()?.replace(',', ".").parse::<f64>().ok();
    }
    None
}

fn security_label_or_value(key: &str, val_lower: &str) -> bool {
    val_lower.contains("wpa")
        || val_lower.contains("wep")
        || val_lower.contains("802.1x")
        || val_lower == "open"
        || key.contains("authentication")
}

/// Parses `netsh wlan show interfaces` output without relying on English
/// labels: SSID/BSSID stay acronyms in every locale, and signal/channel/rate/
/// authentication values keep their shapes ("85%", a bare channel number,
/// "802.11n", "WPA2-Personal", "72,2 MBit/s").
/// Returns None when no connected interface block is present.
fn parse_interfaces_output(text: &str) -> Option<WifiDetails> {
    // netsh separates one interface per blank-line block; parse each and
    // keep the first block that has both SSID and BSSID (the connected one).
    for block in text.split("\n\n") {
        let mut ssid = String::new();
        let mut bssid = String::new();
        let mut signal_percent = 0u32;
        let mut channel = 0u32;
        let mut band = String::new();
        let mut security = String::new();
        let mut link_speed = 0f64;
        let mut interface_name = String::new();

        for line in block.lines() {
            let Some((key, val)) = split_line(line) else {
                continue;
            };
            if val.is_empty() {
                continue;
            }
            if key.starts_with("bssid") {
                bssid = val;
            } else if key.starts_with("ssid") {
                ssid = val;
            } else if let Some(p) = parse_pct(&val) {
                signal_percent = p;
            } else if let Some(c) = parse_channel(&val) {
                channel = c;
            } else if val.starts_with("802.11") {
                band = val;
            } else if security_label_or_value(&key, &val.to_lowercase()) {
                security = val;
            } else if let Some(r) = rate_value(&key, &val) {
                link_speed = link_speed.max(r);
            } else if key == "name" {
                // Only matched on English/German ("Name"); Indonesian "Nama"
                // keeps the default below. Cosmetic only, not telemetry.
                interface_name = val;
            }
        }

        if ssid.is_empty() || bssid.is_empty() {
            continue;
        }

        let mut details = WifiDetails {
            is_connected: true,
            ssid,
            bssid,
            signal_percent,
            channel,
            band: if band.is_empty() { "Unknown".to_string() } else { band },
            link_speed_mbps: link_speed as u32,
            interface_name: if interface_name.is_empty() { "Wi-Fi".to_string() } else { interface_name },
            security: if security.is_empty() { "Unknown".to_string() } else { security },
            ..Default::default()
        };

        // Infer band from channel when the radio-type line was absent
        if details.channel > 0 && details.band == "Unknown" {
            if details.channel <= 14 {
                details.band = "2.4 GHz".to_string();
            } else if details.channel <= 177 {
                details.band = "5 GHz".to_string();
            } else {
                details.band = "6 GHz (Wi-Fi 6E/7)".to_string();
            }
        }

        details.health_verdict = if details.signal_percent >= 80 {
            "Excellent wireless signal with strong throughput.".to_string()
        } else if details.signal_percent >= 60 {
            "Good wireless link. Minor interference may occasionally occur.".to_string()
        } else if details.signal_percent >= 40 {
            "Fair signal strength. Expect latency spikes; consider moving closer or switching to 5GHz/Ethernet.".to_string()
        } else {
            "Weak Wi-Fi signal (<40%). High probability of wireless frame drops and lag spikes.".to_string()
        };

        return Some(details);
    }
    None
}

/// Parses `netsh wlan show networks mode=bssid` output by value shapes and the
/// unlocalized SSID/BSSID acronyms. Signal/channel are read only inside a BSSID
/// block, so SSID-header counters ("Number of BSS entries : 2", French
/// "Taux de base (Mbits/s) : 6 12 24") can never masquerade as channels.
fn parse_networks_output(text: &str, your_bssid: &str) -> Vec<WifiNeighbor> {
    let mut neighbors: Vec<WifiNeighbor> = Vec::new();
    let mut current_ssid = String::new();
    let mut current_security = String::new();
    let mut current_bssid = String::new();
    let mut current_signal: u32 = 0;
    let mut current_channel: u32 = 0;
    let mut in_bss_block = false;

    let mut flush = |ssid: &str, bssid: &str, signal: u32, channel: u32, security: &str| {
        if bssid.is_empty() {
            return;
        }
        let band = channel_to_band(channel);
        let is_current = bssid.eq_ignore_ascii_case(your_bssid);
        neighbors.push(WifiNeighbor {
            ssid: ssid.to_string(),
            bssid: bssid.to_string(),
            signal_percent: signal,
            channel,
            band,
            security: security.to_string(),
            is_current,
        });
    };

    for line in text.lines() {
        let Some((key, val)) = split_line(line) else {
            continue;
        };
        if val.is_empty() {
            continue;
        }
        if key.starts_with("bssid") {
            flush(&current_ssid, &current_bssid, current_signal, current_channel, &current_security);
            current_bssid = val;
            current_signal = 0;
            current_channel = 0;
            in_bss_block = true;
        } else if key.starts_with("ssid") {
            flush(&current_ssid, &current_bssid, current_signal, current_channel, &current_security);
            current_ssid = val;
            current_security.clear();
            in_bss_block = false;
        } else if security_label_or_value(&key, &val.to_lowercase()) {
            current_security = val;
        } else if in_bss_block {
            if let Some(p) = parse_pct(&val) {
                current_signal = p;
            } else if let Some(c) = parse_channel(&val) {
                current_channel = c;
            }
        }
    }
    flush(&current_ssid, &current_bssid, current_signal, current_channel, &current_security);

    neighbors.sort_by(|a, b| {
        b.is_current.cmp(&a.is_current).then(b.signal_percent.cmp(&a.signal_percent))
    });
    neighbors
}

/// Retrieves real-time Wi-Fi connection metrics from the Windows WLAN subsystem using `netsh wlan show interfaces`.
pub fn get_wifi_details() -> WifiDetails {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let output = match Command::new("netsh")
            .args(["wlan", "show", "interfaces"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => return WifiDetails::default(),
        };

        parse_interfaces_output(&output).unwrap_or_default()
    }

    #[cfg(not(target_os = "windows"))]
    {
        WifiDetails::default()
    }
}

/// Scans the Wi-Fi environment: your connection + all visible neighboring networks + channel congestion map.
pub fn scan_wifi_environment() -> WifiEnvironmentScan {
    let your_network = get_wifi_details();

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        use std::collections::HashMap;

        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let output = match Command::new("netsh")
            .args(["wlan", "show", "networks", "mode=bssid"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => {
                return WifiEnvironmentScan {
                    your_network,
                    neighbors: Vec::new(),
                    channel_congestion: Vec::new(),
                    interference_verdict: "Unable to scan Wi-Fi environment.".to_string(),
                }
            }
        };

        let neighbors = parse_networks_output(&output, &your_network.bssid);

        // Build channel congestion map
        let mut chan_map: HashMap<u32, (String, u32, u32)> = HashMap::new(); // channel -> (band, count, strongest_signal)
        for n in &neighbors {
            let entry = chan_map.entry(n.channel).or_insert_with(|| (n.band.clone(), 0, 0));
            entry.1 += 1;
            if n.signal_percent > entry.2 {
                entry.2 = n.signal_percent;
            }
        }

        let mut channel_congestion: Vec<ChannelCongestion> = chan_map.into_iter().map(|(ch, (band, count, sig))| {
            ChannelCongestion {
                channel: ch,
                band,
                network_count: count,
                strongest_signal: sig,
                is_your_channel: your_network.channel == ch && your_network.is_connected,
            }
        }).collect();
        channel_congestion.sort_by_key(|c| c.channel);

        // Generate interference verdict
        let your_ch_count = channel_congestion.iter()
            .find(|c| c.is_your_channel)
            .map(|c| c.network_count)
            .unwrap_or(0);

        // Count overlapping channels in 2.4 GHz (channels within ±4 overlap)
        let overlapping_networks = if your_network.is_connected && your_network.channel <= 14 {
            neighbors.iter().filter(|n| {
                !n.is_current && n.channel <= 14
                    && (n.channel as i32 - your_network.channel as i32).unsigned_abs() < 5
            }).count()
        } else {
            0
        };

        let interference_verdict = if !your_network.is_connected {
            "No active Wi-Fi connection to analyze.".to_string()
        } else if your_ch_count <= 1 && overlapping_networks == 0 {
            format!("✅ Excellent - Channel {} is clear with no co-channel or overlapping interference.", your_network.channel)
        } else if your_ch_count <= 2 && overlapping_networks <= 1 {
            format!("🟢 Good - Channel {} has minimal congestion ({} co-channel, {} overlapping). Performance should be stable.", your_network.channel, your_ch_count.saturating_sub(1), overlapping_networks)
        } else if your_ch_count <= 4 && overlapping_networks <= 3 {
            format!("🟡 Moderate - Channel {} has {} co-channel networks and {} overlapping. Consider switching to a less crowded channel.", your_network.channel, your_ch_count.saturating_sub(1), overlapping_networks)
        } else {
            format!("🔴 Congested - Channel {} is heavily crowded ({} co-channel, {} overlapping). Switch to 5 GHz or a non-overlapping channel (1, 6, or 11 for 2.4 GHz).", your_network.channel, your_ch_count.saturating_sub(1), overlapping_networks)
        };

        WifiEnvironmentScan {
            your_network,
            neighbors,
            channel_congestion,
            interference_verdict,
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        WifiEnvironmentScan {
            your_network,
            neighbors: Vec::new(),
            channel_congestion: Vec::new(),
            interference_verdict: "Wi-Fi environment scanning is only supported on Windows.".to_string(),
        }
    }
}

fn channel_to_band(ch: u32) -> String {
    if ch == 0 {
        "Unknown".to_string()
    } else if ch <= 14 {
        "2.4 GHz".to_string()
    } else if ch <= 177 {
        "5 GHz".to_string()
    } else {
        "6 GHz".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ground truth captured on a de-DE Windows install: every label is
    /// translated, yet the value shapes (acronyms, %, bare channel, 802.11x,
    /// WPA2-*, comma-decimal MBit/s) are identical to en-US.
    const GERMAN_INTERFACES: &str = "\
 Name : Wi-Fi
 Beschreibung : Intel(R) Wi-Fi 6E AX211 160MHz
 Anwendung : [1]
 State : verbunden
 BSSID : aa:bb:cc:dd:ee:ff
 SSID : Testnetz
 Radiotyp : 802.11ax
 Kanal : 36
 Signal : 74%
 Übertragungsrate : 57,8 MBit/s
 Empfangsrate : 72,2 MBit/s
 Authentifizierung : WPA2-Personal
 Verschlüsselung : CCMP
 Verbindungsmodus : Auto
";

    /// An interface block that is present but disconnected has no SSID/BSSID.
    const GERMAN_DISCONNECTED: &str = "\
 Name : Wi-Fi
 Beschreibung : Intel(R) Wi-Fi 6E AX211 160MHz
 State : nicht verbunden
";

    #[test]
    fn parses_german_labels_by_value_shape() {
        let d = parse_interfaces_output(GERMAN_INTERFACES).expect("connected block");
        assert!(d.is_connected);
        assert_eq!(d.ssid, "Testnetz");
        assert_eq!(d.bssid, "aa:bb:cc:dd:ee:ff");
        assert_eq!(d.signal_percent, 74);
        assert_eq!(d.channel, 36);
        assert_eq!(d.band, "802.11ax");
        assert_eq!(d.security, "WPA2-Personal");
        assert_eq!(d.link_speed_mbps, 72);
    }

    #[test]
    fn disconnected_localized_block_is_not_connected() {
        assert!(parse_interfaces_output(GERMAN_DISCONNECTED).is_none());
    }

    #[test]
    fn english_block_still_parses() {
        const EN: &str = "\
 Name : Wi-Fi
 Description : Intel(R) Wi-Fi 6 AX201
 State : connected
 SSID : HomeNet
 BSSID : 11:22:33:44:55:66
 Radio type : 802.11n
 Channel : 6
 Received signal strength : 60%
 Transmission rate : 72.2 Mbps
 Authentication : WPA2-Personal
 Encryption : CCMP
";
        let d = parse_interfaces_output(EN).expect("connected block");
        assert_eq!(d.channel, 6);
        assert_eq!(d.signal_percent, 60);
        assert_eq!(d.interface_name, "Wi-Fi");
    }

    #[test]
    fn rate_comma_decimal_parsed() {
        assert_eq!(parse_rate_mbps("57,8 MBit/s"), Some(57.8));
        assert_eq!(parse_rate_mbps("72.2 Mbps"), Some(72.2));
        assert_eq!(parse_rate_mbps("Not available"), None);
    }

    #[test]
    fn channel_shape_bounds() {
        assert_eq!(parse_channel("6"), Some(6));
        assert_eq!(parse_channel("0"), None); // "PeerCount : 0" must not match
        assert_eq!(parse_channel("260662466610937500"), None); // timestamps
        assert_eq!(parse_channel("85%"), None);
    }

    /// Live-shaped fr-FR capture (this machine's codepage mangles accents to
    /// U+FFFD when read as UTF-8; the parser must be indifferent to that).
    /// French netsh also puts NO-BREAK SPACE before colons (typography rule).
    #[test]
    fn parses_french_labels_with_nbsp_and_unit_in_label() {
        const FR: &str = "\
    Nom                  : Wi-Fi
    Description          : Intel(R) Wi-Fi 6 AX201 160MHz
    GUID                 : 393f9901-b599-41be-8c88-30114b6775fd
    \u{fffd}tat                 : connect\u{fffd}
    SSID                 : TestSSID
    BSSID                : 9a:a9:42:cf:2a:a9
    Type de r\u{fffd}seau       : Infrastructure
    Type de radio        : 802.11ax
    Authentification     : WPA3-Personnel  (H2E)
    Chiffrement          : CCMP
    Canal              \u{a0}: 36
    R\u{fffd}ception (Mbits/s)  : 459
    Transmission (Mbits/s) : 344
    Signal                : 86%
    Profil               : TestSSID
";
        let d = parse_interfaces_output(FR).expect("connected block");
        assert_eq!(d.ssid, "TestSSID");
        assert_eq!(d.bssid, "9a:a9:42:cf:2a:a9");
        assert_eq!(d.channel, 36);
        assert_eq!(d.signal_percent, 86);
        assert_eq!(d.band, "802.11ax");
        assert!(d.security.starts_with("WPA3-Personnel"));
        assert_eq!(d.link_speed_mbps, 459);
    }

    #[test]
    fn scan_french_rates_do_not_masquerade_as_channel() {
        const FR_SCAN: &str = "\
SSID 1\u{a0}: TestSSID
    Type de r\u{fffd}seau\u{a0}         : Infrastructure
    Authentification\u{a0}       : WPA3-Personnel
    Chiffrement\u{a0}            : CCMP
    BSSID 1            \u{a0}: 9a:a9:42:cf:2a:a9
         Signal            \u{a0}: 86%
         Type de radio     \u{a0}: 802.11ax
         Canal              \u{a0}: 36
         Taux de base (Mbits/s)\u{a0}: 6 12 24
         Autres taux (Mbits/s)\u{a0}: 9 18 36 48 54
";
        let n = parse_networks_output(FR_SCAN, "9A:A9:42:CF:2A:A9");
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].channel, 36);
        assert_eq!(n[0].signal_percent, 86);
        assert_eq!(n[0].security, "WPA3-Personnel");
        assert!(n[0].is_current);
    }

    #[test]
    fn scan_english_header_counter_ignored() {        const EN_SCAN: &str = "\
SSID 1 : HomeNet
    Network type              : Infrastructure
    Authentication            : WPA2-Personal
    Encryption                : CCMP
    Number of BSS entries     : 2
    BSSID 1 : aa:bb:cc:dd:ee:ff
         Signal : 55%
         Channel : 11
    BSSID 2 : 11:22:33:44:55:66
         Signal : 40%
         Channel : 1
";
        let n = parse_networks_output(EN_SCAN, "");
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].channel, 11);
        assert_eq!(n[1].channel, 1);
        assert_eq!(n[0].security, "WPA2-Personal");
    }

    /// Manual ground-truth probe: prints live telemetry for comparison with
    /// `netsh wlan show interfaces`. Run with: cargo test --lib -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_machine_check() {
        let d = get_wifi_details();
        println!("wifi details: {d:?}");
        let s = scan_wifi_environment();
        println!("neighbors: {} | verdict: {}", s.neighbors.len(), s.interference_verdict);
        let t1 = crate::engine::throughput::ThroughputMonitor::get_throughput();
        #[cfg(target_os = "windows")]
        for l in crate::engine::throughput::if_table_debug_lines() {
            println!("row: {l}");
        }
        println!("t1 rx={} tx={} iface={}", t1.total_rx_bytes, t1.total_tx_bytes, t1.interface_name);
        std::thread::sleep(std::time::Duration::from_millis(700));
        let t2 = crate::engine::throughput::ThroughputMonitor::get_throughput();
        println!("t2 down={:.1} kbps up={:.1} kbps", t2.download_kbps, t2.upload_kbps);
        assert!(d.is_connected, "expected a live Wi-Fi connection");
        assert!(d.channel >= 1 && d.channel <= 230);
        assert!(d.signal_percent > 0);
        assert!(t1.total_rx_bytes > 0, "MIB octet counters should be non-zero");
    }
}
