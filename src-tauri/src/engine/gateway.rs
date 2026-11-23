use std::net::Ipv4Addr;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetBestRoute, MIB_IPFORWARDROW,
};

/// Resolves the default gateway IPv4 address using Windows GetBestRoute API
pub fn get_default_gateway() -> Option<String> {
    unsafe {
        // Query best route to 8.8.8.8 (public internet)
        let public_target = u32::from_ne_bytes([8, 8, 8, 8]);
        let mut forward_row: MIB_IPFORWARDROW = std::mem::zeroed();
        
        let res = GetBestRoute(public_target, 0, &mut forward_row);
        if res == 0 {
            let next_hop = forward_row.dwForwardNextHop;
            let bytes = next_hop.to_ne_bytes();
            let ip = Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]);
            if !ip.is_unspecified() {
                return Some(ip.to_string());
            }
        }
        None
    }
}
