pub fn calculate_packet_loss(sent: usize, received: usize) -> f64 {
    if sent == 0 {
        return 0.0;
    }
    ((sent - received) as f64 / sent as f64) * 100.0
}

/// Computes RFC 3550 interarrival jitter from a sequence of RTT measurements (ms).
/// Jitter = mean difference between consecutive successful packets.
pub fn calculate_jitter_ms(rtts: &[f64]) -> f64 {
    if rtts.len() < 2 {
        return 0.0;
    }
    let mut sum_diff = 0.0;
    for i in 1..rtts.len() {
        sum_diff += (rtts[i] - rtts[i - 1]).abs();
    }
    sum_diff / ((rtts.len() - 1) as f64)
}

/// Computes a comprehensive Quality Score (0–100) and Letter Grade (A+, A, B, C, D, F)
/// factoring in packet loss percentage and jitter.
pub fn calculate_quality_score(loss_percent: f64, avg_rtt_ms: f64, jitter_ms: f64) -> (u32, &'static str) {
    let mut score = 100.0f64;

    // Packet loss penalty: each 1% of loss costs 8 points
    score -= loss_percent * 8.0;

    // Latency penalty: latency above 60ms incurs penalties (under 60ms is good for internet routing)
    if avg_rtt_ms > 60.0 {
        score -= ((avg_rtt_ms - 60.0) / 5.0).min(25.0);
    }

    // Jitter penalty: jitter under 15ms is normal across different target tiers (e.g. gateway 2ms vs public DNS 30ms)
    if jitter_ms > 15.0 {
        score -= ((jitter_ms - 15.0) / 2.0).min(30.0);
    }

    let clamped = score.clamp(0.0, 100.0).round() as u32;

    let grade = match clamped {
        95..=100 => "A+",
        88..=94 => "A",
        78..=87 => "B",
        65..=77 => "C",
        45..=64 => "D",
        _ => "F",
    };

    (clamped, grade)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_loss_calculation() {
        assert_eq!(calculate_packet_loss(10, 10), 0.0);
        assert_eq!(calculate_packet_loss(10, 8), 20.0);
        assert_eq!(calculate_packet_loss(10, 0), 100.0);
        assert_eq!(calculate_packet_loss(0, 0), 0.0);
    }

    #[test]
    fn test_jitter_calculation() {
        let stable = vec![20.0, 20.0, 20.0];
        assert_eq!(calculate_jitter_ms(&stable), 0.0);

        let jittery = vec![10.0, 30.0, 10.0, 30.0];
        // differences: |30-10|=20, |10-30|=20, |30-10|=20 -> avg = 20.0
        assert_eq!(calculate_jitter_ms(&jittery), 20.0);
    }

    #[test]
    fn test_quality_score_grading() {
        let (perfect_score, grade) = calculate_quality_score(0.0, 15.0, 1.0);
        assert!(perfect_score >= 97);
        assert_eq!(grade, "A+");

        let (loss_score, loss_grade) = calculate_quality_score(15.0, 50.0, 10.0);
        assert!(loss_score < 50);
        assert_eq!(loss_grade, "F");
    }
}
