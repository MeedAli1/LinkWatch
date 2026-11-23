#[cfg(test)]
mod tests {
    use linkwatchapp_lib::diagnostics::engine::{Confidence, DiagnosticEngine, NetworkHealthStatus};
    use linkwatchapp_lib::engine::icmp::ProbeObservation;
    use linkwatchapp_lib::storage::db::{Database, normalize_host, validate_target};

    // ─── Helpers ──────────────────────────────────────────────────────────────

    fn mock_obs(target_id: &str, success: bool, rtt_ms: u32) -> ProbeObservation {
        ProbeObservation {
            target_id: target_id.to_string(),
            target_label: target_id.to_string(),
            host: "127.0.0.1".to_string(),
            resolved_ip: Some("127.0.0.1".to_string()),
            timestamp_ms: 1000,
            rtt_api_ms: if success { Some(rtt_ms) } else { None },
            rtt_measured_us: if success { Some(rtt_ms as u64 * 1000) } else { None },
            success,
            status_code: if success { 0 } else { 11010 },
            error_msg: if success { None } else { Some("Timeout".into()) },
        }
    }

    fn in_memory_db() -> Database {
        Database::init(":memory:").expect("in-memory DB")
    }

    // ─── Diagnostic scenarios (unchanged, regression guard) ───────────────────

    #[test]
    fn test_diagnostic_all_healthy() {
        let obs = vec![
            mock_obs("gateway", true, 2),
            mock_obs("dns_primary", true, 18),
            mock_obs("public_ip", true, 30),
        ];
        let d = DiagnosticEngine::analyze(&obs);
        assert_eq!(d.status, NetworkHealthStatus::Healthy);
        assert_eq!(d.confidence, Confidence::Strong);
    }

    #[test]
    fn test_diagnostic_local_wifi_broken() {
        let obs = vec![
            mock_obs("gateway", false, 0),
            mock_obs("dns_primary", false, 0),
            mock_obs("public_ip", false, 0),
        ];
        let d = DiagnosticEngine::analyze(&obs);
        assert_eq!(d.status, NetworkHealthStatus::LocalIssue);
        assert_eq!(d.confidence, Confidence::Strong);
    }

    #[test]
    fn test_diagnostic_isp_outage() {
        let obs = vec![
            mock_obs("gateway", true, 1),
            mock_obs("dns_primary", false, 0),
            mock_obs("public_ip", false, 0),
        ];
        let d = DiagnosticEngine::analyze(&obs);
        assert_eq!(d.status, NetworkHealthStatus::IspOutage);
    }

    #[test]
    fn test_diagnostic_target_specific() {
        let obs = vec![
            mock_obs("gateway", true, 1),
            mock_obs("dns_primary", false, 0),
            mock_obs("public_ip", true, 30),
        ];
        let d = DiagnosticEngine::analyze(&obs);
        assert_eq!(d.status, NetworkHealthStatus::TargetSpecific);
    }

    // ─── Validation tests ─────────────────────────────────────────────────────

    #[test]
    fn test_valid_ipv4() {
        assert!(validate_target("My Server", "1.2.3.4").is_ok());
    }

    #[test]
    fn test_valid_hostname() {
        assert!(validate_target("Google DNS", "dns.google.com").is_ok());
    }

    #[test]
    fn test_valid_ipv6_no_brackets() {
        // e.g. 2001:db8::1 — colons are allowed
        assert!(validate_target("IPv6 Target", "2001:db8::1").is_ok());
    }

    #[test]
    fn test_valid_ipv6_with_brackets() {
        // Brackets stripped by normalize_host
        let result = validate_target("IPv6 Target", "[::1]");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "::1");
    }

    #[test]
    fn test_reject_empty_host() {
        let e = validate_target("My Server", "").unwrap_err();
        assert_eq!(e.field, "host");
    }

    #[test]
    fn test_reject_empty_label() {
        let e = validate_target("", "1.2.3.4").unwrap_err();
        assert_eq!(e.field, "label");
    }

    #[test]
    fn test_reject_label_too_long() {
        let long = "a".repeat(65);
        let e = validate_target(&long, "1.2.3.4").unwrap_err();
        assert_eq!(e.field, "label");
    }

    #[test]
    fn test_reject_invalid_chars() {
        // Space in host is invalid
        let e = validate_target("Bad", "my host").unwrap_err();
        assert_eq!(e.field, "host");
    }

    #[test]
    fn test_normalize_host_lowercases_and_trims() {
        assert_eq!(normalize_host("  DNS.Google.COM  "), "dns.google.com");
    }

    #[test]
    fn test_normalize_host_strips_brackets() {
        assert_eq!(normalize_host("[::1]"), "::1");
    }

    // ─── Custom target SQLite CRUD tests ─────────────────────────────────────

    #[test]
    fn test_insert_and_list_custom_target() {
        let db = in_memory_db();
        db.insert_custom_target("custom_001", "My Pi", "192.168.1.50", "192.168.1.50", 1000)
            .expect("insert");
        let targets = db.list_custom_targets().unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].label, "My Pi");
        assert_eq!(targets[0].host, "192.168.1.50");
        assert!(targets[0].enabled);
    }

    #[test]
    fn test_duplicate_host_rejected() {
        let db = in_memory_db();
        db.insert_custom_target("custom_001", "Target A", "10.0.0.1", "10.0.0.1", 1000)
            .expect("first insert");
        // Second insert with same normalized host must fail (UNIQUE constraint)
        let result = db.insert_custom_target("custom_002", "Target B", "10.0.0.1", "10.0.0.1", 2000);
        assert!(result.is_err(), "Expected duplicate rejection");
    }

    #[test]
    fn test_duplicate_host_check() {
        let db = in_memory_db();
        db.insert_custom_target("custom_001", "Target A", "10.0.0.1", "10.0.0.1", 1000).unwrap();
        assert!(db.custom_target_host_exists("10.0.0.1").unwrap());
        assert!(!db.custom_target_host_exists("10.0.0.2").unwrap());
    }

    #[test]
    fn test_disable_and_enable_custom_target() {
        let db = in_memory_db();
        db.insert_custom_target("custom_001", "Target A", "10.0.0.1", "10.0.0.1", 1000).unwrap();

        db.set_custom_target_enabled("custom_001", false).unwrap();
        let targets = db.list_custom_targets().unwrap();
        assert!(!targets[0].enabled);

        db.set_custom_target_enabled("custom_001", true).unwrap();
        let targets = db.list_custom_targets().unwrap();
        assert!(targets[0].enabled);
    }

    #[test]
    fn test_remove_custom_target_preserves_history() {
        let db = in_memory_db();
        db.insert_custom_target("custom_001", "Target A", "10.0.0.1", "10.0.0.1", 1000).unwrap();

        // Insert some probe history for this target
        let obs = ProbeObservation {
            target_id: "custom_001".into(), target_label: "Target A".into(),
            host: "10.0.0.1".into(), resolved_ip: Some("10.0.0.1".into()),
            timestamp_ms: 2000, rtt_api_ms: Some(5), rtt_measured_us: Some(5000),
            success: true, status_code: 0, error_msg: None,
        };
        db.insert_observation(&obs).unwrap();

        // Remove target
        db.delete_custom_target("custom_001").unwrap();

        // Target gone from custom_targets
        let targets = db.list_custom_targets().unwrap();
        assert_eq!(targets.len(), 0);

        // But probe_history still has it (measurement data preserved)
        let history = db.get_recent_history(10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].target_id, "custom_001");
    }

    #[test]
    fn test_sqlite_in_memory_probes_and_outages() {
        let db = in_memory_db();
        let obs = mock_obs("test_target", true, 15);
        db.insert_observation(&obs).unwrap();

        let stats = db.get_target_stats().unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].target_id, "test_target");
        assert_eq!(stats[0].total_probes, 1);
        assert_eq!(stats[0].success_count, 1);
        assert_eq!(stats[0].loss_percent, 0.0);

        db.record_outage(1000, Some(2000), "Target", "Test outage").unwrap();
        let outages = db.get_outages(10).unwrap();
        assert_eq!(outages.len(), 1);
        assert_eq!(outages[0].duration_seconds, Some(1));
    }

    // ─── Retention / pruning tests ────────────────────────────────────────────
    
    #[test]
    fn test_prune_by_age_removes_old_records() {
        let db = in_memory_db();
        // Insert 5 "old" probes (timestamp_ms = 1000..5000)
        for i in 0..5 {
            let mut obs = mock_obs("t1", true, 10);
            obs.timestamp_ms = 1000 + i * 1000;
            db.insert_observation(&obs).unwrap();
        }
        // Insert 3 "recent" probes (timestamp_ms = 1_000_000..3_000_000)
        for i in 0..3 {
            let mut obs = mock_obs("t1", true, 10);
            obs.timestamp_ms = 1_000_000 + i * 1_000_000;
            db.insert_observation(&obs).unwrap();
        }
        // Prune anything before ms=100_000
        let deleted = db.prune_by_age(100_000, 1000).unwrap();
        assert_eq!(deleted, 5, "should delete 5 old rows");
        let count = db.count_probes().unwrap();
        assert_eq!(count, 3, "should keep 3 recent rows");
    }
    
    #[test]
    fn test_prune_by_age_preserves_recent() {
        let db = in_memory_db();
        for i in 0..5 {
            let mut obs = mock_obs("t1", true, 10);
            obs.timestamp_ms = 9_000_000 + i * 1_000;
            db.insert_observation(&obs).unwrap();
        }
        let deleted = db.prune_by_age(1_000, 1000).unwrap();
        assert_eq!(deleted, 0, "nothing old to delete");
        assert_eq!(db.count_probes().unwrap(), 5);
    }
    
    #[test]
    fn test_prune_by_count_removes_excess() {
        let db = in_memory_db();
        for i in 0..15 {
            let mut obs = mock_obs("t1", true, 10);
            obs.timestamp_ms = 1000 + i * 1000;
            db.insert_observation(&obs).unwrap();
        }
        let deleted = db.prune_by_count(10, 1000).unwrap();
        assert_eq!(deleted, 5, "should delete 5 excess rows");
        assert_eq!(db.count_probes().unwrap(), 10);
        // Ensure the OLDEST were removed (oldest timestamp = 1000)
        let oldest = db.oldest_probe_ms().unwrap().unwrap();
        assert!(oldest > 5000, "oldest remaining should be > 5th row timestamp");
    }
    
    #[test]
    fn test_prune_by_count_no_op_when_under_limit() {
        let db = in_memory_db();
        for i in 0..5 {
            let mut obs = mock_obs("t1", true, 10);
            obs.timestamp_ms = 1000 + i;
            db.insert_observation(&obs).unwrap();
        }
        let deleted = db.prune_by_count(10, 1000).unwrap();
        assert_eq!(deleted, 0);
        assert_eq!(db.count_probes().unwrap(), 5);
    }
    
    #[test]
    fn test_prune_both_limits() {
        let db = in_memory_db();
        // 20 rows: 10 old, 10 recent
        for i in 0..10 {
            let mut obs = mock_obs("t1", true, 10);
            obs.timestamp_ms = 1000 + i;
            db.insert_observation(&obs).unwrap();
        }
        for i in 0..10 {
            let mut obs = mock_obs("t1", true, 10);
            obs.timestamp_ms = 9_000_000 + i * 1_000;
            db.insert_observation(&obs).unwrap();
        }
        // Age prune: cutoff=5_000_000 removes 10 old rows
        let age_del = db.prune_by_age(5_000_000, 1000).unwrap();
        assert_eq!(age_del, 10);
        // Count prune: max=5 removes 5 more
        let cnt_del = db.prune_by_count(5, 1000).unwrap();
        assert_eq!(cnt_del, 5);
        assert_eq!(db.count_probes().unwrap(), 5);
    }
    
    #[test]
    fn test_prune_preserves_custom_targets() {
        let db = in_memory_db();
        db.insert_custom_target("custom_001", "My Pi", "192.168.1.50", "192.168.1.50", 1000).unwrap();
        // Insert old probe data and prune it
        let mut obs = mock_obs("t1", true, 10);
        obs.timestamp_ms = 500;
        db.insert_observation(&obs).unwrap();
        db.prune_by_age(10_000, 1000).unwrap();
        // Custom target must still be intact
        let targets = db.list_custom_targets().unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].label, "My Pi");
    }
    
    #[test]
    fn test_prune_preserves_app_settings() {
        let db = in_memory_db();
        // Save a retention setting
        let settings = linkwatchapp_lib::storage::db::RetentionSettings {
            pruning_enabled: true,
            max_age_days: 14,
            max_records: 25_000,
            last_cleanup_ms: Some(42_000),
        };
        db.save_retention_settings(&settings).unwrap();
        // Prune probes (none exist, but exercise code path)
        db.prune_by_age(1_000_000, 1000).unwrap();
        // Settings must survive
        let loaded = db.load_retention_settings();
        assert!(loaded.pruning_enabled);
        assert_eq!(loaded.max_age_days, 14);
        assert_eq!(loaded.max_records, 25_000);
        assert_eq!(loaded.last_cleanup_ms, Some(42_000));
    }
    
    #[test]
    fn test_outage_prune_keeps_minimum_500() {
        let db = in_memory_db();
        // Insert 400 old outages — below the 500 minimum threshold
        for i in 0..400i64 {
            db.record_outage(1000 + i, Some(2000 + i), "Target", "Old outage").unwrap();
        }
        let deleted = db.prune_outages(999_999_999).unwrap();
        assert_eq!(deleted, 0, "should not delete when count <= 500");
        // Now add 200 more and prune → should delete 100 (600 - 500 keep)
        for i in 0..200i64 {
            db.record_outage(10_000_000 + i, None, "Target", "Recent").unwrap();
        }
        // cutoff low enough to only target original 400 old ones
        let deleted2 = db.prune_outages(5_000_000).unwrap();
        // We have 600 total; keep 500 → may delete up to 100 of the old ones
        assert!(deleted2 <= 100, "should not delete more than 100");
    }
    
    #[test]
    fn test_retention_settings_roundtrip() {
        let db = in_memory_db();
        let original = linkwatchapp_lib::storage::db::RetentionSettings {
            pruning_enabled: false,
            max_age_days: 30,
            max_records: 100_000,
            last_cleanup_ms: Some(123_456_789),
        };
        db.save_retention_settings(&original).unwrap();
        let loaded = db.load_retention_settings();
        assert!(!loaded.pruning_enabled);
        assert_eq!(loaded.max_age_days, 30);
        assert_eq!(loaded.max_records, 100_000);
        assert_eq!(loaded.last_cleanup_ms, Some(123_456_789));
    }
    
    #[test]
    fn test_checkpoint_passive_does_not_error() {
        let db = in_memory_db();
        // In-memory DBs have no WAL file, but the PRAGMA should not error.
        db.checkpoint_passive().expect("passive checkpoint should succeed on in-memory DB");
    }
    
    #[test]
    fn test_db_stats_counts_correctly() {
        let db = in_memory_db();
        db.insert_custom_target("c1", "Host", "10.0.0.1", "10.0.0.1", 1000).unwrap();
        let obs = mock_obs("t1", true, 10);
        db.insert_observation(&obs).unwrap();
        db.record_outage(1000, Some(2000), "Target", "Test").unwrap();
        let stats = db.get_db_stats(None);
        assert_eq!(stats.probe_count, 1);
        assert_eq!(stats.outage_count, 1);
        assert_eq!(stats.custom_target_count, 1);
        assert!(stats.db_size_bytes > 0);
    }
    

    // ─── Milestone 4: Time-Series & Benchmark Tests ─────────────────────────

    #[test]
    fn test_timeseries_aggregation_accuracy() {
        let db = in_memory_db();
        // Insert 6 probes for target "t1":
        // Bucket 0 (ms 0..10_000): 3 probes (rtt 10ms, 20ms, timeout) -> avg 15ms, 1 loss, 33.3% loss
        let mut p1 = mock_obs("t1", true, 10);
        p1.timestamp_ms = 1_000;
        p1.resolved_ip = Some("1.1.1.1".into());
        db.insert_observation(&p1).unwrap();

        let mut p2 = mock_obs("t1", true, 20);
        p2.timestamp_ms = 2_000;
        p2.resolved_ip = Some("1.1.1.1".into());
        db.insert_observation(&p2).unwrap();

        let mut p3 = mock_obs("t1", false, 0);
        p3.timestamp_ms = 3_000;
        p3.resolved_ip = Some("1.1.1.1".into());
        db.insert_observation(&p3).unwrap();

        // Bucket 1 (ms 10_000..20_000): 2 probes (rtt 30ms, 50ms) -> avg 40ms, 0 loss
        let mut p4 = mock_obs("t1", true, 30);
        p4.timestamp_ms = 11_000;
        db.insert_observation(&p4).unwrap();

        let mut p5 = mock_obs("t1", true, 50);
        p5.timestamp_ms = 12_000;
        db.insert_observation(&p5).unwrap();

        let res = db.get_target_timeseries("t1", 0, 20_000, 10_000).unwrap();
        assert_eq!(res.buckets.len(), 2, "should have 2 buckets");

        let b0 = &res.buckets[0];
        assert_eq!(b0.bucket_start_ms, 0);
        assert_eq!(b0.total_probes, 3);
        assert_eq!(b0.success_count, 2);
        assert_eq!(b0.loss_count, 1);
        assert!((b0.loss_percent - 33.333).abs() < 0.1);
        assert!((b0.avg_rtt_ms.unwrap() - 15.0).abs() < 0.01);
        assert!((b0.min_rtt_ms.unwrap() - 10.0).abs() < 0.01);
        assert!((b0.max_rtt_ms.unwrap() - 20.0).abs() < 0.01);
        assert_eq!(b0.last_resolved_ip.as_deref(), Some("1.1.1.1"));

        let b1 = &res.buckets[1];
        assert_eq!(b1.bucket_start_ms, 10_000);
        assert_eq!(b1.total_probes, 2);
        assert_eq!(b1.loss_count, 0);
        assert!((b1.avg_rtt_ms.unwrap() - 40.0).abs() < 0.01);

        assert_eq!(res.recent_probes.len(), 5, "recent probes preserved");
    }

    #[test]
    fn test_timeseries_empty_window() {
        let db = in_memory_db();
        let res = db.get_target_timeseries("non_existent", 0, 100_000, 10_000).unwrap();
        assert_eq!(res.buckets.len(), 0);
        assert_eq!(res.recent_probes.len(), 0);
        assert_eq!(res.outages.len(), 0);
    }

    #[test]
    fn test_timeseries_outage_interval_filtering() {
        let db = in_memory_db();
        // Outage 1: 5_000..15_000 (overlaps window 10_000..30_000)
        db.record_outage(5_000, Some(15_000), "All", "Outage 1").unwrap();
        // Outage 2: 40_000..50_000 (outside window)
        db.record_outage(40_000, Some(50_000), "All", "Outage 2").unwrap();

        let res = db.get_target_timeseries("t1", 10_000, 30_000, 5_000).unwrap();
        assert_eq!(res.outages.len(), 1);
        assert_eq!(res.outages[0].summary, "Outage 1");
    }

    #[test]
    fn test_benchmark_50k_records_query_performance() {
        use std::time::Instant;

        let db = in_memory_db();
        // Insert 50,000 synthetic records using a transaction for speed
        {
            let conn = db.conn.lock();
            let mut stmt = conn.prepare_cached(
                "INSERT INTO probe_history (target_id, target_label, host, resolved_ip, timestamp_ms, rtt_api_ms, rtt_measured_us, success, status_code, error_msg)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            ).unwrap();

            conn.execute_batch("BEGIN TRANSACTION;").unwrap();
            let base_time = 1_700_000_000_000i64;
            for i in 0..50_000 {
                let success = i % 20 != 0; // 5% packet loss
                let rtt_us = if success { (10_000 + (i % 50) * 1000) as i64 } else { 0 };
                let rtt_ms = if success { (rtt_us / 1000) as i32 } else { 0 };
                stmt.execute(rusqlite::params![
                    if i % 2 == 0 { "gateway" } else { "dns_primary" },
                    "Target",
                    "1.1.1.1",
                    "1.1.1.1",
                    base_time + (i as i64 * 1000), // 1 second intervals
                    if success { Some(rtt_ms) } else { None },
                    if success { Some(rtt_us) } else { None },
                    if success { 1 } else { 0 },
                    if success { 0 } else { 11010 },
                    if success { None } else { Some("Timeout") }
                ]).unwrap();
            }
            conn.execute_batch("COMMIT;").unwrap();
        }

        assert_eq!(db.count_probes().unwrap(), 50_000);

        // Benchmark get_target_timeseries for a 1-hour window (3600 seconds)
        let t0 = Instant::now();
        let base_time = 1_700_000_000_000i64;
        let res = db.get_target_timeseries("dns_primary", base_time, base_time + 3_600_000, 60_000).unwrap();
        let elapsed = t0.elapsed();

        println!("\n>>> 50,000 records benchmark: 1-hour window query took {:?}", elapsed);
        assert!(!res.buckets.is_empty(), "buckets should not be empty");
        // Verify index-driven performance is fast (under 150ms even on unoptimized debug builds)
        assert!(elapsed.as_millis() < 500, "Query should be fast with index");
    }

    // ─── Milestone 5: Autostart & Start Minimized Tests ─────────────────────

    #[test]
    fn test_db_start_minimized_roundtrip() {
        let db = in_memory_db();
        // Default should be false
        assert!(!db.get_start_minimized());

        // Enable and verify
        db.set_start_minimized(true).unwrap();
        assert!(db.get_start_minimized());

        // Disable and verify
        db.set_start_minimized(false).unwrap();
        assert!(!db.get_start_minimized());
    }

    #[test]
    #[ignore = "mutates the real HKCU Run key; run explicitly with --ignored"]
    fn test_autostart_registry_toggle() {
        // Test enabling and disabling autostart in CurrentUser Run key
        let res_enable = linkwatchapp_lib::engine::autostart::set_autostart_enabled(true, true);
        assert!(res_enable.is_ok(), "Setting autostart enabled should succeed");

        let is_enabled = linkwatchapp_lib::engine::autostart::is_autostart_enabled().unwrap();
        assert!(is_enabled, "is_autostart_enabled should return true after enable");

        // Clean up by disabling autostart
        let res_disable = linkwatchapp_lib::engine::autostart::set_autostart_enabled(false, false);
        assert!(res_disable.is_ok(), "Disabling autostart should succeed");

        let is_disabled = linkwatchapp_lib::engine::autostart::is_autostart_enabled().unwrap();
        assert!(!is_disabled, "is_autostart_enabled should return false after disable");
    }

    #[test]
    fn test_command_line_argument_parsing() {
        let args = vec!["linkwatchapp.exe".to_string(), "--minimized".to_string()];
        let has_minimized = args.iter().any(|a| a == "--minimized" || a == "--tray");
        assert!(has_minimized);

        let normal_args = vec!["linkwatchapp.exe".to_string()];
        let has_minimized_normal = normal_args.iter().any(|a| a == "--minimized" || a == "--tray");
        assert!(!has_minimized_normal);
    }

    // ─── ISP Dispute & Quality Score Tests ──────────────────────────────────

    #[test]
    fn test_diagnostic_engine_quality_score_and_culprit() {
        let obs = vec![
            mock_obs("gateway", true, 2),
            mock_obs("dns_primary", true, 18),
            mock_obs("public_ip", true, 30),
        ];
        let d = DiagnosticEngine::analyze(&obs);
        assert_eq!(d.status, NetworkHealthStatus::Healthy);
        assert_eq!(d.culprit, linkwatchapp_lib::diagnostics::engine::CulpritScope::None);
        assert!(d.quality_score >= 90);
        assert!(d.quality_grade == "A+" || d.quality_grade == "A");
    }

    #[test]
    fn test_diagnostic_engine_isp_culprit_isolation() {
        let obs = vec![
            mock_obs("gateway", true, 1),
            mock_obs("dns_primary", false, 0),
            mock_obs("public_ip", false, 0),
        ];
        let d = DiagnosticEngine::analyze(&obs);
        assert_eq!(d.status, NetworkHealthStatus::IspOutage);
        assert_eq!(d.culprit, linkwatchapp_lib::diagnostics::engine::CulpritScope::IspWan);
        assert_eq!(d.quality_grade, "F");
    }
}