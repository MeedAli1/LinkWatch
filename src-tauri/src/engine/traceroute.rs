use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::net::IpAddr;
use std::time::Instant;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, IP_OPTION_INFORMATION,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracerouteHop {
    pub hop_number: u32,
    pub ip: Option<String>,
    pub hostname: Option<String>,
    pub rtt_ms: Option<f64>,
    pub status: String,
    pub is_destination: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracerouteResult {
    pub target: String,
    pub destination_ip: String,
    pub hops: Vec<TracerouteHop>,
    pub total_hops: u32,
    pub completed: bool,
}

/// Performs an incremental TTL multi-hop traceroute up to max_hops.
pub fn trace_route(target: &str, max_hops: u8) -> TracerouteResult {
    let dest_ip: IpAddr = match target.parse::<IpAddr>() {
        Ok(ip) => ip,
        Err(_) => {
            // Resolve host
            use std::net::ToSocketAddrs;
            match format!("{}:80", target).to_socket_addrs() {
                Ok(mut addrs) => match addrs.next() {
                    Some(sa) => sa.ip(),
                    None => {
                        return TracerouteResult {
                            target: target.to_string(),
                            destination_ip: "Unable to resolve".to_string(),
                            hops: Vec::new(),
                            total_hops: 0,
                            completed: false,
                        }
                    }
                },
                Err(_) => {
                    return TracerouteResult {
                        target: target.to_string(),
                        destination_ip: "Unable to resolve".to_string(),
                        hops: Vec::new(),
                        total_hops: 0,
                        completed: false,
                    }
                }
            }
        }
    };

    let dest_ipv4 = match dest_ip {
        IpAddr::V4(v4) => u32::from_ne_bytes(v4.octets()),
        IpAddr::V6(_) => {
            return TracerouteResult {
                target: target.to_string(),
                destination_ip: dest_ip.to_string(),
                hops: Vec::new(),
                total_hops: 0,
                completed: false,
            }
        }
    };

    let handle: HANDLE = unsafe { IcmpCreateFile() };
    if handle.is_null() {
        return TracerouteResult {
            target: target.to_string(),
            destination_ip: dest_ip.to_string(),
            hops: Vec::new(),
            total_hops: 0,
            completed: false,
        };
    }

    let mut hops = Vec::new();
    let mut completed = false;

    // Buffer for ICMP reply
    let reply_size = std::mem::size_of::<crate::engine::icmp::IcmpEchoReply>() + 64;
    let mut reply_buffer = vec![0u8; reply_size];
    let send_data = [b'L', b'i', b'n', b'k', b'W', b'a', b't', b'c', b'h'];

    for ttl in 1..=max_hops {
        let mut ip_options = IP_OPTION_INFORMATION {
            Ttl: ttl,
            Tos: 0,
            Flags: 0,
            OptionsSize: 0,
            OptionsData: std::ptr::null_mut(),
        };

        let start = Instant::now();
        let ret = unsafe {
            IcmpSendEcho(
                handle,
                dest_ipv4,
                send_data.as_ptr() as *const c_void,
                send_data.len() as u16,
                &mut ip_options,
                reply_buffer.as_mut_ptr() as *mut c_void,
                reply_size as u32,
                1200, // 1.2s timeout per hop
            )
        };
        let rtt_ms = start.elapsed().as_secs_f64() * 1000.0;

        if ret > 0 {
            let reply = unsafe { &*(reply_buffer.as_ptr() as *const crate::engine::icmp::IcmpEchoReply) };
            let reply_ip = std::net::Ipv4Addr::from(reply.address.to_ne_bytes()).to_string();

            // Status 0 = Success (Reached destination), 11013 = TTL expired in transit (intermediate router hop)
            let is_dest = reply.status == 0 || reply_ip == dest_ip.to_string();

            hops.push(TracerouteHop {
                hop_number: ttl as u32,
                ip: Some(reply_ip),
                hostname: None,
                rtt_ms: Some(rtt_ms),
                status: if is_dest { "Destination Reached".to_string() } else { "Transit Hop".to_string() },
                is_destination: is_dest,
            });

            if is_dest {
                completed = true;
                break;
            }
        } else {
            hops.push(TracerouteHop {
                hop_number: ttl as u32,
                ip: None,
                hostname: None,
                rtt_ms: None,
                status: "Request timed out (*)".to_string(),
                is_destination: false,
            });
        }
    }

    unsafe {
        let _ = IcmpCloseHandle(handle);
    }

    let total = hops.len() as u32;

    TracerouteResult {
        target: target.to_string(),
        destination_ip: dest_ip.to_string(),
        hops,
        total_hops: total,
        completed,
    }
}
