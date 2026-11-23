use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanDevice {
    pub ip: String,
    pub mac: String,
    pub vendor: String,
    pub device_type: String,
    pub hostname: String,
    pub is_gateway: bool,
    pub is_current_host: bool,
    pub ping_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanScanResult {
    pub subnet: String,
    pub devices: Vec<LanDevice>,
    pub total_devices_found: usize,
    pub scanned_at_ms: i64,
}

fn identify_vendor(mac: &str) -> (&'static str, &'static str) {
    let clean = mac.to_uppercase().replace([':', '-'], "");
    if clean.len() < 6 {
        return ("Unknown", "Generic Device");
    }
    let oui = &clean[..6];

    // Check if locally administered / randomized MAC (bit 1 of first byte is 1)
    if let Ok(first_byte) = u8::from_str_radix(&clean[..2], 16) {
        if (first_byte & 0x02) != 0 {
            return ("Private / Randomized MAC", "Mobile Device (iOS / Android / Windows)");
        }
    }

    match oui {
        "F01898" | "BC9FEF" | "A483E7" | "3C15C2" | "D89695" | "ACDE48" | "703EAC" | "68DBCA"
        | "ACCF23" | "B8E856" | "40A3CC" | "9C35EB" | "186590" | "88665A" => {
            ("Apple Inc.", "iPhone / iPad / Mac")
        }
        "3482C5" | "508569" | "5CE91E" | "94652D" | "A87D12" | "C0BDD1"
        | "4844F7" | "606BBD" | "842519" | "DC7144" => {
            ("Samsung Electronics", "Galaxy Smartphone / Smart TV")
        }
        "CC1531" | "001500" | "001B21" | "00216A" | "3413E8" | "4851B7" | "8086F2" | "A0369F" => {
            ("Intel Corp.", "Laptop / PC Network Adapter")
        }
        "98A942" => ("Tozed Kangwei", "4G/5G CPE Router Gateway"),
        "50C7BF" | "60E327" | "B04E26" | "E848B8" | "EC086B" | "F4F26D" | "704F57" | "14CC20" => {
            ("TP-Link", "Wi-Fi Router / Access Point / Smart Plug")
        }
        "00041F" | "001315" | "0019C5" | "709E29" | "A8E3EE" | "FC0F4B" | "00D9D1" => {
            ("Sony Interactive", "PlayStation Console")
        }
        "00155D" | "281878" | "6045BD" | "7C1E52" | "DCB4C4" | "B827EB" => {
            ("Microsoft Corp.", "Surface / Xbox / Windows PC")
        }
        "F4F5E8" | "546009" | "6C2995" | "B4FBE4" | "A47733" | "3C5AB4" => {
            ("Google LLC", "Chromecast / Pixel / Nest Hub")
        }
        "B44506" | "00E04C" | "525400" => ("Realtek Semiconductor", "PC Ethernet Controller"),
        "286C07" | "50EC50" | "AC1702" => ("Xiaomi Communications", "Mi Phone / Smart Home"),
        "4401BB" | "7C6193" | "F88A5E" => ("Huawei Technologies", "Smartphone / Router"),
        _ => ("Local Device", "Network Client"),
    }
}

/// Resolve PTR names for the given addresses on detached threads with a hard global
/// deadline. getnameinfo has no timeout parameter, so instead of waiting on slow
/// resolvers we stop collecting after `max_total_ms` and let stragglers finish
/// unheard; threads only send on a channel, so late replies cannot corrupt output.
/// Returns the resolved names keyed by IP string; unresolved IPs keep their
/// fallback device names.
fn resolve_hostnames_parallel(
    addrs: &[std::net::IpAddr],
    max_total_ms: u64,
) -> HashMap<String, String> {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    let (tx, rx) = mpsc::channel();
    for ip in addrs {
        let tx = tx.clone();
        let ip = *ip;
        let _ = std::thread::Builder::new()
            .stack_size(192 * 1024)
            .spawn(move || {
                let name = dns_lookup::lookup_addr(&ip)
                    .ok()
                    .filter(|n| !n.is_empty() && n != &ip.to_string());
                let _ = tx.send((ip.to_string(), name));
            });
    }
    drop(tx);

    let mut out = HashMap::new();
    let deadline = Instant::now() + Duration::from_millis(max_total_ms);
    while out.len() < addrs.len() {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        match rx.recv_timeout(deadline - now) {
            Ok((ip, Some(name))) => {
                out.insert(ip, name);
            }
            Ok((_, None)) => {
                // Count the failed lookup so we do not keep waiting for it.
                out.insert(String::new(), String::new());
            }
            Err(_) => break, // timed out or all senders gone
        }
    }
    out.retain(|k, _| !k.is_empty());
    out
}

/// Discovers connected devices on the local LAN using the Windows ARP cache table (`arp -a`)
/// and identifies device hostnames via DNS / reverse lookup and NetBIOS.
pub fn scan_local_lan() -> LanScanResult {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let output = match Command::new("arp")
            .arg("-a")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => {
                return LanScanResult {
                    subnet: "Local Subnet".to_string(),
                    devices: Vec::new(),
                    total_devices_found: 0,
                    scanned_at_ms: chrono::Utc::now().timestamp_millis(),
                }
            }
        };

        let gw = crate::engine::gateway::get_default_gateway().unwrap_or_default();
        let current_hostname = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This PC".to_string());
        let mut devices = Vec::new();
        // (device index, ip) for the deferred parallel reverse-DNS pass
        let mut ptr_pending: Vec<(usize, std::net::IpAddr)> = Vec::new();

        for line in output.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 && parts[0].contains('.') && parts[1].contains('-') {
                let ip_str = parts[0].to_string();
                let mac_raw = parts[1].to_uppercase();

                // Skip broadcast / multicast / loopback
                if ip_str.ends_with(".255")
                    || ip_str.starts_with("224.")
                    || ip_str.starts_with("239.")
                    || ip_str.starts_with("255.")
                    || mac_raw == "FF-FF-FF-FF-FF-FF"
                {
                    continue;
                }

                let mac_str = mac_raw.replace('-', ":");
                let is_gateway = ip_str == gw;

                let (vendor, dev_type) = if is_gateway {
                    ("Tozed / Router Gateway", "Default Gateway / Wi-Fi Router")
                } else {
                    identify_vendor(&mac_str)
                };

                // Determine user-friendly Device Name (Hostname). Reverse DNS is
                // deferred to the parallel pass below, so start with a vendor-based
                // fallback that stays if the lookup is slow or fails.
                let hostname = if is_gateway {
                    "Main Wi-Fi Gateway / Router".to_string()
                } else if vendor.contains("Randomized") {
                    "Wireless Client (Private MAC)".to_string()
                } else if vendor != "Local Device" {
                    format!("{} Device", vendor.split_whitespace().next().unwrap_or(vendor))
                } else {
                    format!("LAN Client ({})", dev_type)
                };

                if !is_gateway && ptr_pending.len() < 32 {
                    if let Ok(ip) = ip_str.parse::<std::net::IpAddr>() {
                        ptr_pending.push((devices.len(), ip));
                    }
                }

                devices.push(LanDevice {
                    ip: ip_str,
                    mac: mac_str,
                    vendor: vendor.to_string(),
                    device_type: dev_type.to_string(),
                    hostname,
                    is_gateway,
                    is_current_host: false,
                    ping_ms: None,
                });
            }
        }

        // Resolve hostnames concurrently under one hard deadline, replacing the old
        // serial per-device lookup that could stall the scan on slow resolvers.
        if !ptr_pending.is_empty() {
            let addrs: Vec<std::net::IpAddr> = ptr_pending.iter().map(|(_, ip)| *ip).collect();
            let names = resolve_hostnames_parallel(&addrs, 1500);
            for (idx, ip) in &ptr_pending {
                if let Some(name) = names.get(&ip.to_string()) {
                    devices[*idx].hostname = name.clone();
                }
            }
        }

        // Add this local PC to the devices list if known Wi-Fi IP is found
        if let Ok(ip_out) = Command::new("powershell")
            .args(["-NoProfile", "-Command", "Get-NetIPAddress -InterfaceAlias 'Wi-Fi*' -AddressFamily IPv4 | Select-Object -ExpandProperty IPAddress"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            let my_ip = String::from_utf8_lossy(&ip_out.stdout).trim().to_string();
            if !my_ip.is_empty() && my_ip.contains('.') && !devices.iter().any(|d| d.ip == my_ip) {
                devices.push(LanDevice {
                    ip: my_ip,
                    mac: "Local Interface".to_string(),
                    vendor: "Intel / Host PC".to_string(),
                    device_type: "This Computer (Current Host)".to_string(),
                    hostname: format!("{} (This PC)", current_hostname),
                    is_gateway: false,
                    is_current_host: true,
                    ping_ms: Some(0.1),
                });
            }
        }

        devices.sort_by(|a, b| {
            if a.is_current_host {
                std::cmp::Ordering::Less
            } else if b.is_current_host {
                std::cmp::Ordering::Greater
            } else if a.is_gateway {
                std::cmp::Ordering::Less
            } else if b.is_gateway {
                std::cmp::Ordering::Greater
            } else {
                a.ip.cmp(&b.ip)
            }
        });

        let total = devices.len();

        LanScanResult {
            subnet: if !gw.is_empty() { format!("{}/24", gw) } else { "Local Subnet".to_string() },
            devices,
            total_devices_found: total,
            scanned_at_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        LanScanResult {
            subnet: "192.168.1.0/24".to_string(),
            devices: Vec::new(),
            total_devices_found: 0,
            scanned_at_ms: chrono::Utc::now().timestamp_millis(),
        }
    }
}