# Architecture

LinkWatch is structured in three layers:

1. **Engine** (`src-tauri/src/engine/`) — one module per diagnostic or platform capability
2. **Service** (`src-tauri/src/service.rs`) — the always-running monitor that drives probes, evaluates alert rules, and persists results
3. **Frontend** (`src/`) — React UI that polls snapshots over Tauri IPC and issues commands

```
src-tauri/src/
  lib.rs              Tauri entry, IPC commands, tray + notification wiring
  service.rs          Background monitor: probe loop, alerts, persistence, export
  storage/
    db.rs             SQLite schema, migrations, queries, CSV export
    retention.rs      Data-pruning worker
  engine/
    icmp.rs           IPv4 ICMP pinger (IcmpSendEcho)
    traceroute.rs     Traceroute engine
    dns_bench.rs      DNS server benchmarking
    bufferbloat.rs    QoS load testing
    lan_scanner.rs    Local network scan
    gateway.rs        Gateway diagnostics
    wifi_inspector.rs Wi-Fi interface + channel analysis
    throughput.rs     Per-target throughput sampling
    network_repair.rs Windows network stack repair
    hotkeys.rs        Hotkey config persistence
    overlay_config.rs Mini-HUD config persistence
    autostart.rs      Startup registry integration
src/
  App.tsx             Main window UI, state management, charts
  Overlay.tsx         Floating mini-HUD window
  audioChimes.ts      Alert sound generation
```

The probe worker runs on its own thread, parallelizes ICMP probes, caches DNS results, and writes observation batches each cycle.

## Data model

The SQLite database (`src-tauri/src/storage/db.rs`) contains four tables: `probe_history` (per-target latency/jitter/loss samples), `outages` (detected down periods), `custom_targets` (user-added hosts), and `app_settings` (key/value preferences). The database runs in WAL mode, and startup includes corrupt-database recovery.
