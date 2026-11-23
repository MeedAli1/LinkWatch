pub mod diagnostics;
pub mod engine;
pub mod service;
pub mod storage;

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::service::{AppStateSnapshot, MonitorService};
use crate::storage::db::{CustomTarget, Database, DbStats, RetentionSettings, TargetTimeSeriesResult};

/// Set when a corrupt history DB was quarantined and recreated at startup;
/// setup() reads it once to surface a notification (none exists yet pre-window).
static DB_RECOVERED: AtomicBool = AtomicBool::new(false);

pub struct AppContext {
    pub service: Arc<MonitorService>,
}

#[tauri::command]
fn start_monitoring(state: State<AppContext>) -> Result<(), String> {
    state.service.start();
    Ok(())
}

#[tauri::command]
async fn stop_monitoring(state: State<'_, AppContext>) -> Result<(), String> {
    // Async + spawn_blocking: stop() joins the probe worker, and a sync
    // command would run that join on the event-loop thread - the whole UI
    // freezes until it returns, which WebView2 treats as a crash.
    let service = state.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.stop())
        .await
        .map_err(|e| format!("stop failed: {e}"))?;
    Ok(())
}

#[tauri::command]
fn get_snapshot(state: State<AppContext>) -> Result<AppStateSnapshot, String> {
    Ok(state.service.get_snapshot())
}

#[tauri::command]
fn export_csv_data(state: State<AppContext>) -> Result<String, String> {
    state.service.export_csv()
}

#[tauri::command]
fn export_html_report_data(state: State<AppContext>) -> Result<String, String> {
    state.service.export_html_report()
}

#[tauri::command]
fn export_isp_dispute_report(state: State<AppContext>) -> Result<String, String> {
    state.service.export_isp_dispute_report()
}

#[tauri::command]
async fn run_dns_benchmark() -> Result<crate::engine::dns_bench::DnsBenchmarkResult, String> {
    let res = tauri::async_runtime::spawn_blocking(crate::engine::dns_bench::run_dns_benchmark)
        .await
        .map_err(|e| e.to_string())?;
    Ok(res)
}

#[tauri::command]
async fn get_wifi_details() -> Result<crate::engine::wifi_inspector::WifiDetails, String> {
    let res = tauri::async_runtime::spawn_blocking(crate::engine::wifi_inspector::get_wifi_details)
        .await
        .map_err(|e| e.to_string())?;
    Ok(res)
}

#[tauri::command]
async fn scan_wifi_environment() -> Result<crate::engine::wifi_inspector::WifiEnvironmentScan, String> {
    let res = tauri::async_runtime::spawn_blocking(crate::engine::wifi_inspector::scan_wifi_environment)
        .await
        .map_err(|e| e.to_string())?;
    Ok(res)
}

#[tauri::command]
async fn run_traceroute(target: String, max_hops: u8) -> Result<crate::engine::traceroute::TracerouteResult, String> {
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::engine::traceroute::trace_route(&target, max_hops)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(res)
}

#[tauri::command]
fn trigger_audio_alert(is_outage: bool) -> Result<(), String> {
    crate::engine::audio_alerts::play_audio_alert(is_outage);
    Ok(())
}

#[tauri::command]
async fn get_adapter_throughput() -> Result<crate::engine::throughput::ThroughputData, String> {
    let res = tauri::async_runtime::spawn_blocking(crate::engine::throughput::ThroughputMonitor::get_throughput)
        .await
        .map_err(|e| e.to_string())?;
    Ok(res)
}

#[tauri::command]
async fn run_game_radar() -> Result<crate::engine::game_radar::GameRadarResult, String> {
    let res = tauri::async_runtime::spawn_blocking(crate::engine::game_radar::scan_game_radar)
        .await
        .map_err(|e| e.to_string())?;
    Ok(res)
}

#[tauri::command]
async fn scan_local_lan() -> Result<crate::engine::lan_scanner::LanScanResult, String> {
    let res = tauri::async_runtime::spawn_blocking(crate::engine::lan_scanner::scan_local_lan)
        .await
        .map_err(|e| e.to_string())?;
    Ok(res)
}

#[tauri::command]
async fn run_bufferbloat_test() -> Result<crate::engine::bufferbloat::BufferbloatResult, String> {
    let res = tauri::async_runtime::spawn_blocking(crate::engine::bufferbloat::run_bufferbloat_test)
        .await
        .map_err(|e| e.to_string())??;
    Ok(res)
}

#[tauri::command]
async fn execute_network_repair() -> Result<crate::engine::network_repair::NetworkRepairResult, String> {
    let res = tauri::async_runtime::spawn_blocking(crate::engine::network_repair::execute_network_repair)
        .await
        .map_err(|e| e.to_string())?;
    Ok(res)
}

#[tauri::command]
async fn open_overlay(app: tauri::AppHandle) -> Result<(), String> {
    crate::engine::overlay::open_overlay(&app)?;
    let _ = app.emit("overlay-visibility-changed", true);
    Ok(())
}

#[tauri::command]
async fn close_overlay(app: tauri::AppHandle) -> Result<(), String> {
    crate::engine::overlay::close_overlay(&app)?;
    let _ = app.emit("overlay-visibility-changed", false);
    Ok(())
}

#[tauri::command]
fn get_overlay_config() -> Result<crate::engine::overlay_config::OverlayConfig, String> {
    Ok(crate::engine::overlay_config::load_overlay_config())
}

#[tauri::command]
fn save_overlay_config_cmd(
    app: tauri::AppHandle,
    config: crate::engine::overlay_config::OverlayConfig,
) -> Result<(), String> {
    crate::engine::overlay_config::save_overlay_config(&config)?;
    crate::engine::overlay::reposition_overlay(&app, &config);
    let _ = app.emit("overlay-config-updated", &config);
    Ok(())
}

#[tauri::command]
fn add_custom_target(
    state: State<AppContext>,
    label: String,
    host: String,
) -> Result<CustomTarget, String> {
    state.service.add_custom_target(&label, &host)
}

#[tauri::command]
fn set_custom_target_enabled(
    state: State<AppContext>,
    target_id: String,
    enabled: bool,
) -> Result<(), String> {
    state.service.set_custom_target_enabled(&target_id, enabled)
}

#[tauri::command]
fn remove_custom_target(
    state: State<AppContext>,
    target_id: String,
) -> Result<(), String> {
    state.service.remove_custom_target(&target_id)
}

#[tauri::command]
fn get_db_stats(state: State<AppContext>) -> Result<DbStats, String> {
    Ok(state.service.get_db_stats())
}

#[tauri::command]
fn get_retention_settings(state: State<AppContext>) -> Result<RetentionSettings, String> {
    Ok(state.service.get_retention_settings())
}

#[tauri::command]
fn save_retention_settings(
    state: State<AppContext>,
    settings: RetentionSettings,
) -> Result<(), String> {
    state.service.save_retention_settings(settings)
}

#[tauri::command]
fn run_cleanup_now(state: State<AppContext>) -> Result<DbStats, String> {
    Ok(state.service.run_cleanup_now())
}

#[tauri::command]
fn get_target_timeseries(
    state: State<AppContext>,
    target_id: String,
    start_ms: i64,
    end_ms: i64,
    bucket_size_ms: i64,
) -> Result<TargetTimeSeriesResult, String> {
    state.service.get_target_timeseries(&target_id, start_ms, end_ms, bucket_size_ms)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AutostartConfig {
    pub autostart_enabled: bool,
    pub start_minimized: bool,
}

#[tauri::command]
fn get_probe_interval(state: State<AppContext>) -> Result<u64, String> {
    Ok(state.service.get_probe_interval_ms())
}

#[tauri::command]
fn set_probe_interval(state: State<AppContext>, interval_ms: u64) -> Result<(), String> {
    state.service.set_probe_interval_ms(interval_ms)
}

#[tauri::command]
fn get_autostart_config(state: State<AppContext>) -> Result<AutostartConfig, String> {
    let autostart_enabled = crate::engine::autostart::is_autostart_enabled().unwrap_or(false);
    let start_minimized = state.service.get_start_minimized();
    Ok(AutostartConfig {
        autostart_enabled,
        start_minimized,
    })
}

#[tauri::command]
fn set_autostart_config(
    state: State<AppContext>,
    autostart_enabled: bool,
    start_minimized: bool,
) -> Result<(), String> {
    state.service.set_start_minimized(start_minimized)?;
    crate::engine::autostart::set_autostart_enabled(autostart_enabled, start_minimized)?;
    Ok(())
}

// ─── Hotkey config commands ──────────────────────────────────────────────────

#[tauri::command]
fn get_hotkey_config() -> Result<crate::engine::hotkeys::HotkeyConfig, String> {
    Ok(crate::engine::hotkeys::load_hotkey_config())
}

#[tauri::command]
fn save_hotkey_config_cmd(
    app: tauri::AppHandle,
    config: crate::engine::hotkeys::HotkeyConfig,
) -> Result<(), String> {
    // 1. Persist to disk
    crate::engine::hotkeys::save_hotkey_config(&config)?;

    // 2. Unregister all existing shortcuts
    app.global_shortcut().unregister_all().map_err(|e| e.to_string())?;

    // 3. Re-register enabled ones
    for binding in &config.bindings {
        if !binding.enabled {
            continue;
        }
        let Some(ref accel) = binding.accelerator else {
            continue;
        };
        let accel_str = accel.clone();
        let action = binding.action.clone();
        let app_handle = app.clone();

        let _ = app.global_shortcut().on_shortcut(accel_str.as_str(), move |_app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                match action {
                    crate::engine::hotkeys::HotkeyAction::ToggleWindow => {
                        if let Some(w) = app_handle.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    }
                    crate::engine::hotkeys::HotkeyAction::ToggleMiniHud => {
                        hotkey_toggle_hud(&app_handle);
                    }
                    crate::engine::hotkeys::HotkeyAction::NetworkRepair => {
                        hotkey_network_repair(&app_handle);
                    }
                    crate::engine::hotkeys::HotkeyAction::OpenDiagnostics => {
                        if let Some(w) = app_handle.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                        let _ = app_handle.emit("hotkey-open-diagnostics", ());
                    }
                }
            }
        });
    }

    Ok(())
}

/// Toggle the HUD from the global hotkey and broadcast the resulting
/// visibility, so the main window's overlay button cannot go stale.
fn hotkey_toggle_hud(app: &tauri::AppHandle) {
    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(shown) = crate::engine::overlay::toggle_overlay(&h) {
            let _ = h.emit("overlay-visibility-changed", shown);
        }
    });
}

/// Start a stack repair from the global hotkey: announce the start so the UI
/// can show its spinner and switch to the tools tab, then run the same repair
/// the command runs and emit its result (or failure) back to the window.
fn hotkey_network_repair(app: &tauri::AppHandle) {
    let h = app.clone();
    let _ = h.emit("hotkey-network-repair", ());
    tauri::async_runtime::spawn(async move {
        match tauri::async_runtime::spawn_blocking(crate::engine::network_repair::execute_network_repair).await {
            Ok(result) => {
                let _ = h.emit("network-repair-result", result);
            }
            Err(e) => {
                let _ = h.emit("hotkey-repair-failed", e.to_string());
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_dir = dirs_or_local();
    let db_path = app_dir.join("linkwatch.db");
    let db = Database::init(&db_path).unwrap_or_else(|first_err| {
        // A half-written DB (power loss mid-write) must not become a silent
        // crash loop: quarantine the bad file, start fresh, notify in setup.
        let bad = app_dir.join(format!(
            "linkwatch.corrupt-{}.db",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        ));
        if std::fs::rename(&db_path, &bad).is_err() {
            let _ = std::fs::remove_file(&db_path);
        }
        eprintln!("LinkWatch: history DB unreadable ({first_err:?}); recreated as {bad:?}");
        DB_RECOVERED.store(true, Ordering::SeqCst);
        Database::init(&db_path)
            .unwrap_or_else(|e| panic!("Failed to initialize fresh SQLite database: {e:?}"))
    });
    let monitor = MonitorService::new(db, db_path.clone()).expect("Failed to create monitor service");
    // Check command line arguments and settings for silent / minimized startup
    let start_minimized_setting = monitor.get_start_minimized();
    let has_minimized_arg = std::env::args().any(|arg| arg == "--minimized" || arg == "--tray");
    let should_start_minimized = start_minimized_setting || has_minimized_arg;

    monitor.start();

    let service_arc = Arc::new(monitor);
    let context = AppContext {
        service: Arc::clone(&service_arc),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .manage(context)
        .setup(move |app| {
            if DB_RECOVERED.load(Ordering::SeqCst) {
                use tauri_plugin_notification::NotificationExt;
                let _ = app.handle().notification()
                    .builder()
                    .title("LinkWatch: History Database Recovered")
                    .body("The previous history file was unreadable, so it was saved aside and monitoring continues with a fresh database.")
                    .show();
            }
            // ── Auto-register saved hotkeys on startup ──────────────────────
            let saved_cfg = crate::engine::hotkeys::load_hotkey_config();
            for binding in &saved_cfg.bindings {
                if !binding.enabled { continue; }
                let Some(ref accel) = binding.accelerator else { continue; };
                let accel_str = accel.clone();
                let action = binding.action.clone();
                let app_handle = app.handle().clone();
                let _ = app.handle().global_shortcut().on_shortcut(
                    accel_str.as_str(),
                    move |_app, _shortcut, event| {
                        if event.state == ShortcutState::Pressed {
                            match action {
                                crate::engine::hotkeys::HotkeyAction::ToggleWindow => {
                                    if let Some(w) = app_handle.get_webview_window("main") {
                                        if w.is_visible().unwrap_or(false) {
                                            let _ = w.hide();
                                        } else {
                                            let _ = w.show(); let _ = w.set_focus();
                                        }
                                    }
                                }
                                crate::engine::hotkeys::HotkeyAction::ToggleMiniHud => {
                                    hotkey_toggle_hud(&app_handle);
                                }
                                crate::engine::hotkeys::HotkeyAction::NetworkRepair => {
                                    hotkey_network_repair(&app_handle);
                                }
                                crate::engine::hotkeys::HotkeyAction::OpenDiagnostics => {
                                    if let Some(w) = app_handle.get_webview_window("main") {
                                        let _ = w.show(); let _ = w.set_focus();
                                    }
                                    let _ = app_handle.emit("hotkey-open-diagnostics", ());
                                }
                            }
                        }
                    },
                );
            }
            let open_item = MenuItem::with_id(app, "open", "Open LinkWatch", true, None::<&str>)?;
            let toggle_item = MenuItem::with_id(app, "toggle_monitor", "Pause Monitoring", true, None::<&str>)?;
            let exit_item = MenuItem::with_id(app, "exit", "Exit LinkWatch", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&open_item, &toggle_item, &exit_item])?;

            let service_for_menu = Arc::clone(&service_arc);
            let toggle_item_ref = toggle_item.clone();

            let tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("LinkWatch - Network Diagnostic Monitor")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app_handle, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(w) = app_handle.get_webview_window("main") {
                            let _ = w.show(); let _ = w.unminimize(); let _ = w.set_focus();
                        }
                    }
                    "toggle_monitor" => {
                        let service = Arc::clone(&service_for_menu);
                        let item = toggle_item_ref.clone();
                        // Same reason as the stop_monitoring command: the
                        // worker join must never run on the event loop.
                        std::thread::spawn(move || {
                            if service.is_running() {
                                service.stop();
                                let _ = item.set_text("Resume Monitoring");
                            } else {
                                service.start();
                                let _ = item.set_text("Pause Monitoring");
                            }
                        });
                    }
                    "exit" => {
                        let service = Arc::clone(&service_for_menu);
                        let app = app_handle.clone();
                        std::thread::spawn(move || {
                            // Bounded: shutdown() gives the worker 2.5 s and
                            // start() is locked out meanwhile, so Resume can
                            // no longer resurrect the thread being joined.
                            service.shutdown();
                            app.exit(0);
                            // If the event loop never turns the exit into a
                            // process stop, force it. WAL plus the corrupt-DB
                            // recovery at startup absorbs a hard kill.
                            std::thread::sleep(std::time::Duration::from_secs(4));
                            std::process::exit(0);
                        });
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up, ..
                    } = event {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show(); let _ = w.unminimize(); let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            // ── Background thread: update tray tooltip with live ping stats ─────
            {
                let tray_handle = tray.clone();
                let service_for_tray = Arc::clone(&service_arc);
                let app_handle_for_tray = app.handle().clone();
                std::thread::spawn(move || {
                    use tauri_plugin_notification::NotificationExt;
                    let mut prev_healthy = true;
                    // Consecutive non-Healthy tray checks before announcing an issue.
                    // One transient blip must not fire a desktop notification.
                    let mut unhealthy_streak: u32 = 0;
                    // Latched so the desktop warning fires once per session;
                    // the tooltip repeats it while history is not saving.
                    let mut storage_notified = false;
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(3));

                        if !service_for_tray.is_running() {
                            let _ = tray_handle.set_tooltip(Some("LinkWatch - Monitoring Paused"));
                            continue;
                        }

                        let snap = service_for_tray.get_snapshot();

                        if snap.storage_degraded && !storage_notified {
                            storage_notified = true;
                            let _ = app_handle_for_tray.notification()
                                .builder()
                                .title("⚠️ LinkWatch - History Not Being Recorded")
                                .body("Several probe cycles failed to save. Live charts still work, but nothing new is reaching the database - check free disk space, then restart LinkWatch.")
                                .show();
                        }

                        let status_icon = match snap.diagnostic.status {
                            crate::diagnostics::engine::NetworkHealthStatus::Healthy => "✅",
                            crate::diagnostics::engine::NetworkHealthStatus::Degraded => "⚠️",
                            crate::diagnostics::engine::NetworkHealthStatus::IspOutage => "🔴",
                            crate::diagnostics::engine::NetworkHealthStatus::GatewayIssue => "🔴",
                            crate::diagnostics::engine::NetworkHealthStatus::LocalIssue => "⚠️",
                            _ => "ℹ️",
                        };

                        // Find latest non-gateway RTT average
                        let latest: Vec<_> = snap.latest_observations.iter().rev()
                            .filter(|o| o.target_id != "gateway" && o.success)
                            .collect();
                        let avg_ms = if !latest.is_empty() {
                            let sum: f64 = latest.iter()
                                .filter_map(|o| o.rtt_measured_us.map(|u| u as f64 / 1000.0))
                                .sum();
                            sum / latest.len() as f64
                        } else {
                            0.0
                        };

                        // Gateway RTT
                        let gw_ms = snap.latest_observations.iter().rev()
                            .find(|o| o.target_id == "gateway" && o.success)
                            .and_then(|o| o.rtt_measured_us.map(|u| u as f64 / 1000.0))
                            .unwrap_or(0.0);

                        // Overall loss
                        let total_probes: i64 = snap.stats.iter().map(|s| s.total_probes).sum();
                        let total_success: i64 = snap.stats.iter().map(|s| s.success_count).sum();
                        let loss_pct = if total_probes > 0 {
                            ((total_probes - total_success) as f64 / total_probes as f64) * 100.0
                        } else {
                            0.0
                        };

                        let tooltip = format!(
                            "{} LinkWatch\nGateway: {:.0} ms | Internet: {:.0} ms\nLoss: {:.1}% | {} {}{}",
                            status_icon,
                            gw_ms, avg_ms, loss_pct,
                            snap.diagnostic.quality_grade,
                            snap.diagnostic.title,
                            if snap.storage_degraded { " | HISTORY NOT SAVING" } else { "" }
                        );
                        // Windows tooltip max is ~128 chars
                        let truncated: String = tooltip.chars().take(127).collect();
                        let _ = tray_handle.set_tooltip(Some(&truncated));

                        // ── Desktop notifications on state transitions (debounced) ──
                        let currently_healthy = matches!(snap.diagnostic.status,
                            crate::diagnostics::engine::NetworkHealthStatus::Healthy);

                        if currently_healthy {
                            unhealthy_streak = 0;
                            if !prev_healthy {
                                let _ = app_handle_for_tray.notification()
                                    .builder()
                                    .title("✅ LinkWatch - Connection Restored")
                                    .body("Network health is back to normal.")
                                    .show();
                                prev_healthy = true;
                            }
                        } else if prev_healthy {
                            unhealthy_streak += 1;
                            if unhealthy_streak >= 2 {
                                let _ = app_handle_for_tray.notification()
                                    .builder()
                                    .title("⚠️ LinkWatch - Network Issue Detected")
                                    .body(&format!("{}\n{}", snap.diagnostic.title, snap.diagnostic.observation))
                                    .show();
                                prev_healthy = false;
                            }
                        }
                    }
                });
            }

            if should_start_minimized {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            start_monitoring,
            stop_monitoring,
            get_snapshot,
            export_csv_data,
            export_html_report_data,
            export_isp_dispute_report,
            run_dns_benchmark,
            get_wifi_details,
            scan_wifi_environment,
            run_traceroute,
            trigger_audio_alert,
            get_adapter_throughput,
            run_game_radar,
            scan_local_lan,
            run_bufferbloat_test,
            execute_network_repair,
            open_overlay,
            close_overlay,
            get_overlay_config,
            save_overlay_config_cmd,
            add_custom_target,
            set_custom_target_enabled,
            remove_custom_target,
            get_db_stats,
            get_retention_settings,
            save_retention_settings,
            run_cleanup_now,
            get_target_timeseries,
            get_autostart_config,
            set_autostart_config,
            get_hotkey_config,
            save_hotkey_config_cmd,
            get_probe_interval,
            set_probe_interval,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn dirs_or_local() -> std::path::PathBuf {
    if let Some(mut dir) = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from) {
        dir.push("LinkWatch");
        let _ = fs::create_dir_all(&dir);
        dir
    } else {
        std::path::PathBuf::from(".")
    }
}
