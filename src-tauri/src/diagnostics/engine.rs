use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::diagnostics::calc::{calculate_jitter_ms, calculate_packet_loss, calculate_quality_score};
use crate::engine::icmp::ProbeObservation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    None,
    Weak,
    Moderate,
    Strong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkHealthStatus {
    Healthy,
    Degraded,
    LocalIssue,
    GatewayIssue,
    IspOutage,
    TargetSpecific,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CulpritScope {
    None,
    LocalWifi,
    GatewayRouter,
    IspWan,
    RemoteTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticFinding {
    pub status: NetworkHealthStatus,
    pub culprit: CulpritScope,
    pub title: String,
    pub observation: String,
    pub hypothesis: String,
    pub confidence: Confidence,
    pub recommendation: String,
    pub affected_scope: String,
    pub quality_score: u32,
    pub quality_grade: String,
    pub jitter_ms: f64,
}

pub struct DiagnosticEngine;

impl DiagnosticEngine {
    pub fn analyze(observations: &[ProbeObservation]) -> DiagnosticFinding {
        if observations.is_empty() {
            return DiagnosticFinding {
                status: NetworkHealthStatus::Unknown,
                culprit: CulpritScope::None,
                title: "Awaiting Data".to_string(),
                observation: "No probe observations have been collected yet.".to_string(),
                hypothesis: "Engine is starting up and probing initial targets.".to_string(),
                confidence: Confidence::None,
                recommendation: "Allow a few seconds for the initial probing cycle.".to_string(),
                affected_scope: "System".to_string(),
                quality_score: 100,
                quality_grade: "A+".to_string(),
                jitter_ms: 0.0,
            };
        }

        // Group latest observation per target_id
        let mut latest_by_target: HashMap<&str, &ProbeObservation> = HashMap::new();
        for obs in observations {
            latest_by_target.insert(&obs.target_id, obs);
        }

        // Majority-based down detection: a target counts as down only when more than
        // half of its observations in the supplied window failed. One transient
        // timeout in the latest cycle no longer flips the verdict.
        let mut down_tally: HashMap<&str, (usize, usize)> = HashMap::new();
        for obs in observations {
            let entry = down_tally.entry(obs.target_id.as_str()).or_insert((0, 0));
            entry.1 += 1;
            if !obs.success {
                entry.0 += 1;
            }
        }
        let target_down = |id: &str| -> bool {
            down_tally
                .get(id)
                .map_or(false, |(fails, total)| *fails * 2 > *total)
        };
        let gateway_down = target_down("gateway");
        let dns_down = target_down("dns_primary");
        let pub_down = target_down("public_ip");

        // Quality and jitter calculation across cycle
        let mut rtts: Vec<f64> = Vec::new();
        let mut total_count = 0usize;
        let mut success_count = 0usize;

        for obs in observations {
            total_count += 1;
            if obs.success {
                success_count += 1;
                if let Some(us) = obs.rtt_measured_us {
                    rtts.push(us as f64 / 1000.0);
                }
            }
        }

        let jitter_ms = calculate_jitter_ms(&rtts);
        let loss_percent = calculate_packet_loss(total_count, success_count);
        let avg_rtt = if !rtts.is_empty() {
            rtts.iter().sum::<f64>() / rtts.len() as f64
        } else {
            0.0
        };

        let (quality_score, quality_grade_str) = calculate_quality_score(loss_percent, avg_rtt, jitter_ms);
        let quality_grade = quality_grade_str.to_string();

        // Case 1: Gateway itself is unresponsive
        if gateway_down {
            if dns_down && pub_down {
                return DiagnosticFinding {
                    status: NetworkHealthStatus::LocalIssue,
                    culprit: CulpritScope::LocalWifi,
                    title: "Local Wi-Fi / Router Disconnection".to_string(),
                    observation: "No response from local gateway, DNS, or public targets.".to_string(),
                    hypothesis: "The network adapter is disconnected, Wi-Fi link is broken, or the local router is powered off.".to_string(),
                    confidence: Confidence::Strong,
                    recommendation: "Check physical cable or Wi-Fi connection, and verify your local router/access point is active.".to_string(),
                    affected_scope: "Local Link / LAN".to_string(),
                    quality_score: 0,
                    quality_grade: "F".to_string(),
                    jitter_ms,
                };
            } else {
                return DiagnosticFinding {
                    status: NetworkHealthStatus::GatewayIssue,
                    culprit: CulpritScope::GatewayRouter,
                    title: "Gateway ICMP Unresponsive".to_string(),
                    observation: "Local gateway dropped ICMP echo, but public internet targets are reachable.".to_string(),
                    hypothesis: "Router may be configured to drop ICMP echo requests on the LAN interface or is under high local CPU load.".to_string(),
                    confidence: Confidence::Moderate,
                    recommendation: "No action needed if internet traffic flows; check router firewall ICMP settings if monitoring is desired.".to_string(),
                    affected_scope: "Local Gateway".to_string(),
                    quality_score,
                    quality_grade,
                    jitter_ms,
                };
            }
        }

        // Case 2: Gateway is responsive, but all external internet targets are failing
        if !gateway_down && dns_down && pub_down {
            return DiagnosticFinding {
                status: NetworkHealthStatus::IspOutage,
                culprit: CulpritScope::IspWan,
                title: "Upstream ISP / WAN Outage".to_string(),
                observation: "Local router gateway responds normally, but multiple independent public hosts (1.1.1.1, 8.8.8.8) are unreachable.".to_string(),
                hypothesis: "Your local connection to the router is working, but the modem/ONT or upstream Internet Service Provider (ISP) is down.".to_string(),
                confidence: Confidence::Strong,
                recommendation: "Restart your modem or contact your ISP to check for area outages.".to_string(),
                affected_scope: "Upstream WAN / ISP".to_string(),
                quality_score: 0,
                quality_grade: "F".to_string(),
                jitter_ms,
            };
        }

        // Case 3: Gateway responsive, one public target failing, others succeeding
        if !gateway_down && (dns_down ^ pub_down) {
            let failing = if dns_down { "Primary DNS" } else { "Secondary DNS/CDN" };
            return DiagnosticFinding {
                status: NetworkHealthStatus::TargetSpecific,
                culprit: CulpritScope::RemoteTarget,
                title: format!("Target Specific Degradation ({failing})"),
                observation: format!("{failing} is timing out while gateway and alternate internet targets respond cleanly."),
                hypothesis: "The issue is localized to a specific transit route or destination server, not your general connection.".to_string(),
                confidence: Confidence::Moderate,
                recommendation: "No local network action required; the internet connection itself is functional.".to_string(),
                affected_scope: "Destination / Transit".to_string(),
                quality_score,
                quality_grade,
                jitter_ms,
            };
        }

        // Case 4: Latency & Jitter analysis, on the latest observation per target
        // (the window can span many cycles; an old spike alone must not hold the
        // status Degraded).
        let mut high_rtt = false;
        let mut rtt_summary = String::new();
        for obs in latest_by_target.values() {
            if let Some(rtt_us) = obs.rtt_measured_us {
                let ms = rtt_us as f64 / 1000.0;
                if ms > 150.0 {
                    high_rtt = true;
                    rtt_summary.push_str(&format!("{} ({:.1}ms) ", obs.target_label, ms));
                }
            }
        }

        if high_rtt || jitter_ms > 25.0 {
            return DiagnosticFinding {
                status: NetworkHealthStatus::Degraded,
                culprit: CulpritScope::IspWan,
                title: if jitter_ms > 25.0 { "High Network Jitter Detected".to_string() } else { "Elevated Latency Detected".to_string() },
                observation: format!("Jitter: {:.1}ms. High latency measured on: {}", jitter_ms, rtt_summary),
                hypothesis: "Bufferbloat, Wi-Fi channel interference, or ISP route congestion is degrading real-time call and gaming quality.".to_string(),
                confidence: Confidence::Moderate,
                recommendation: "Check for heavy downloads/updates or enable SQM/QoS on your router.".to_string(),
                affected_scope: "Bandwidth / Bufferbloat".to_string(),
                quality_score,
                quality_grade,
                jitter_ms,
            };
        }

        // Healthy
        DiagnosticFinding {
            status: NetworkHealthStatus::Healthy,
            culprit: CulpritScope::None,
            title: "Network Connection Healthy".to_string(),
            observation: "Consistent responses received from local gateway and global internet targets with low latency and 0% loss.".to_string(),
            hypothesis: "Local LAN, router gateway, ISP WAN, and external routing are all functioning within normal parameters.".to_string(),
            confidence: Confidence::Strong,
            recommendation: "No action required.".to_string(),
            affected_scope: "All Layers".to_string(),
            quality_score,
            quality_grade,
            jitter_ms,
        }
    }
}
