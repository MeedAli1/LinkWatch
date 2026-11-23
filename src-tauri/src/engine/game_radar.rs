use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameRegionPing {
    pub game_title: String,
    pub region_name: String,
    pub endpoint_host: String,
    pub ping_ms: Option<f64>,
    pub quality_tier: String, // "Optimal (<30ms)", "Great (<65ms)", "Playable (<110ms)", "High Ping (>110ms)", "Timeout"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameRadarResult {
    pub pings: Vec<GameRegionPing>,
    pub lowest_ping_region: String,
    pub lowest_ping_ms: f64,
    pub scanned_at_ms: i64,
}

pub fn scan_game_radar() -> GameRadarResult {
    // Verified, high-availability European SDR relays, gaming edge endpoints, and voice clusters (closest lowest-ping regions)
    let targets = [
        ("Counter-Strike 2 / Steam", "Europe Central (Frankfurt)", "155.133.226.68"),
        ("Counter-Strike 2 / Steam", "Europe West (London)", "162.254.196.66"),
        ("Counter-Strike 2 / Steam", "Europe West (Paris)", "185.25.182.18"),
        ("Counter-Strike 2 / Steam", "Europe East (Vienna)", "146.66.155.66"),
        ("Counter-Strike 2 / Steam", "Europe North (Stockholm)", "162.254.198.41"),
        ("Counter-Strike 2 / Steam", "Europe South (Madrid)", "155.133.246.34"),
        ("Counter-Strike 2 / Steam", "Europe East (Warsaw)", "155.133.230.100"),
        ("Valorant / Riot Games", "Europe Central (Frankfurt)", "104.160.142.157"),
        ("Valorant / Riot Games", "Europe West (Dublin)", "104.160.144.20"),
        ("Fortnite / AWS Cloud", "Europe South (Milan)", "dynamodb.eu-south-1.amazonaws.com"),
        ("Fortnite / AWS Cloud", "Europe Central (Frankfurt)", "dynamodb.eu-central-1.amazonaws.com"),
        ("Fortnite / AWS Cloud", "Europe West (Paris)", "dynamodb.eu-west-3.amazonaws.com"),
        ("Fortnite / AWS Cloud", "Europe West (London)", "dynamodb.eu-west-2.amazonaws.com"),
        ("Discord Voice Servers", "Europe Voice Gateway", "gateway.discord.gg"),
        ("Discord Voice Servers", "Europe Low-Latency Edge", "discord.gg"),
        ("Blizzard / Battle.net", "Europe Central (Frankfurt)", "185.60.112.157"),
    ];

    let pinger = match crate::engine::icmp::WinIcmpPinger::new() {
        Ok(p) => p,
        Err(_) => {
            return GameRadarResult {
                pings: Vec::new(),
                lowest_ping_region: "Unavailable".to_string(),
                lowest_ping_ms: 0.0,
                scanned_at_ms: chrono::Utc::now().timestamp_millis(),
            }
        }
    };

    let mut pings = Vec::new();

    for (game, region, host) in targets {
        let obs = pinger.ping("game_radar", region, host, 1200);
        let ping_ms = if obs.success {
            obs.rtt_measured_us.map(|us| (us as f64 / 1000.0 * 10.0).round() / 10.0)
        } else {
            None
        };

        let quality_tier = match ping_ms {
            Some(ms) if ms < 30.0 => "Optimal (<30ms)",
            Some(ms) if ms < 65.0 => "Great (<65ms)",
            Some(ms) if ms < 110.0 => "Playable (<110ms)",
            Some(_) => "High Ping (>110ms)",
            None => "Timeout / Unreachable",
        };

        pings.push(GameRegionPing {
            game_title: game.to_string(),
            region_name: region.to_string(),
            endpoint_host: host.to_string(),
            ping_ms,
            quality_tier: quality_tier.to_string(),
        });
    }

    // Find region with lowest ping
    let mut lowest_ms = f64::MAX;
    let mut lowest_region = "None".to_string();

    for p in &pings {
        if let Some(ms) = p.ping_ms {
            if ms < lowest_ms {
                lowest_ms = ms;
                lowest_region = format!("{} - {}", p.game_title, p.region_name);
            }
        }
    }

    if lowest_ms == f64::MAX {
        lowest_ms = 0.0;
        lowest_region = "All Probes Timed Out".to_string();
    }

    GameRadarResult {
        pings,
        lowest_ping_region: lowest_region,
        lowest_ping_ms: lowest_ms,
        scanned_at_ms: chrono::Utc::now().timestamp_millis(),
    }
}
