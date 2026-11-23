use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputStats {
    pub download_kbps: f64,
    pub upload_kbps: f64,
    pub total_rx_mb: f64,
    pub total_tx_mb: f64,
    pub active_interface: String,
}

impl Default for ThroughputStats {
    fn default() -> Self {
        ThroughputStats {
            download_kbps: 0.0,
            upload_kbps: 0.0,
            total_rx_mb: 0.0,
            total_tx_mb: 0.0,
            active_interface: "Primary Adapter".to_string(),
        }
    }
}

/// Plays a discrete Windows system audio cue for packet loss or outage events.
pub fn play_audio_alert(is_outage: bool) {
    #[cfg(target_os = "windows")]
    {
        extern "system" {
            fn MessageBeep(uType: u32) -> i32;
        }
        const MB_ICONWARNING: u32 = 0x00000030;
        const MB_ICONHAND: u32 = 0x00000010;

        unsafe {
            if is_outage {
                MessageBeep(MB_ICONHAND);
            } else {
                MessageBeep(MB_ICONWARNING);
            }
        }
    }
}