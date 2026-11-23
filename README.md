# LinkWatch

Local-first network quality and diagnostics monitor for Windows.

Windows 10 or later only.

LinkWatch pings your gateway and the internet on a timer, charts latency and jitter over time, detects outages, and gives you one-click diagnostic tools — traceroute, DNS comparison, bufferbloat detection, LAN scan — all from one tray-resident window. No telemetry, no accounts, and your history never leaves your machine. Monitoring and diagnostics run without admin rights; the only step that needs elevation is the ARP-table clear inside network repair.

## Features

**Monitoring**
- Continuous ICMP latency/jitter/packet-loss probing of gateway and internet targets, with parallel probes and cached DNS resolution
- Configurable probe interval and custom targets (edit, enable/disable, delete via the UI or IPC commands)
- Rolling history stored in SQLite, automatic data retention, CSV export
- Outage and degradation alerts with optional audio chimes and a desktop notification
- Wi-Fi link diagnostics: interface info, signal quality, nearby-network scan, channel congestion
- Bufferbloat / QoS load testing on demand
- Throughput sampling per target

**Diagnostics**
- Traceroute with hop latency and loss visualization
- DNS server benchmark and comparison
- LAN scanner for nearby hosts
- Game server radar: pings fixed regional game-server endpoints (Steam/CS2, Riot/Valorant and similar) and reports the lowest-ping region per title
- Gateway diagnostics: resolves the active default gateway's IPv4 address directly from the Windows routing table
- 1-click network repair (flush DNS cache, register DNS, renew DHCP, clear ARP table)

**Interface**
- System tray presence with a live tooltip summary
- Floating always-on-top mini-HUD with configurable position
- Custom global hotkeys
- Windows autostart (with optional minimized launch)
- Theme support and dark/light styling

## What it looks like

**Live dashboard** — per-target latency, jitter, and packet-loss with rolling sparkline charts, a confidence verdict, and summary / root cause / action panels.

![Live dashboard](docs/images/dashboard.png)

**Floating mini-HUD** — an always-on-top, click-through overlay with ping, jitter, loss, health grade, and throughput, visible over any window or borderless game.

![Floating mini-HUD](docs/images/mini-hud.png)

**Tools & diagnostics** — one-click network repair, DNS resolver benchmark, Wi-Fi and channel congestion telemetry, traceroute, and bufferbloat stress testing.

![Tools and diagnostics panel](docs/images/diagnostics.png)

**Game server radar & LAN scan** — pings regional game-server endpoints (CS2, Valorant, Fortnite, Discord) to find your best region, and enumerates active hosts on your subnet.

![Game server radar and LAN scan](docs/images/game-radar.png)

## Tech stack

- **Tauri 2** + Rust — core monitoring engine, tray, storage
- **React 19** + TypeScript — UI
- **Tailwind CSS 4** — styling
- **SQLite** — persisted probe history, outages, targets, settings

## Prerequisites

- Windows 10 or later
- [Rust toolchain](https://www.rust-lang.org/tools/install) with the MSVC ABI and Visual Studio Build Tools (Tauri's Windows prerequisite)
- [Node.js](https://nodejs.org) and npm
- Microsoft Edge WebView2 Runtime (preinstalled on recent Windows 10/11 builds)

## Download

Grab the latest installers from [GitHub Releases](https://github.com/MeedAli1/LinkWatch/releases) — either `LinkWatch_0.1.0_x64_en-US.msi` (Windows Installer) or `LinkWatch_0.1.0_x64-setup.exe` (NSIS setup). Unsigned builds show a Windows SmartScreen warning; choose "More info → Run anyway".

Or build from source below.

## Getting started

```bash
# 1. Install JS dependencies
npm install

# 2. Run in development mode (hot reload)
npm run tauri dev

# 3. Build a release executable
npm run tauri build
```

The release binary lands in `src-tauri/target/release/linkwatchapp.exe`.

## Commands

| Command | Description |
| --- | --- |
| `npm run dev` | Frontend dev server only (Vite, no native shell) |
| `npm run tauri dev` | Full app in development mode |
| `npm run build` | Type-check (`tsc`) and build the frontend bundle |
| `npm run tauri build` | Build the distributable native app |
| `cd src-tauri && cargo test` | Run the Rust unit-test suite |
| `cd src-tauri && cargo test --lib -- --ignored --nocapture` | Run the live-probe network tests |
| `cd src-tauri && cargo fmt` | Format Rust code |
| `cd src-tauri && cargo clippy` | Lint Rust code |

## Configuration

All app state lives in `%LOCALAPPDATA%\LinkWatch\`:

| File | Purpose |
| --- | --- |
| `linkwatch.db` | SQLite database (probe history, outages, targets, settings) |
| `hotkeys.json` | Global hotkey bindings |
| `overlay_config.json` | Mini-HUD position and state |

Default hotkeys (remappable):

| Action | Default |
| --- | --- |
| Toggle main window | `Ctrl+Shift+L` |
| Toggle mini-HUD | `Ctrl+Shift+M` |
| Network repair | unbound |
| Open diagnostics | unbound |

Accelerators use the Tauri global-shortcut format (e.g. `CmdOrCtrl+Shift+L`, `Alt+F9`).

There are no environment variables to configure for normal use.

## Architecture

The app has three layers: an **engine** of one-module-per-capability Rust components (`src-tauri/src/engine/`), a **service** layer running the always-on probe loop, alert rules, and persistence (`src-tauri/src/service.rs`), and a **React frontend** that polls snapshots over Tauri IPC. Probe history, outages, custom targets, and settings are stored in a local SQLite database. Details in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Development notes

- Windows-only: `src-tauri` links Windows system libraries (`windows-sys`) and the build script embeds the app icon as a Windows resource.
- The mini-HUD opens as a separate webview window loading `index.html?overlay=1`.
- `src-tauri/tests/diagnostic_tests.rs` contains integration tests that hit live network targets, so expect environment-dependent results.
- Monitoring needs no admin rights. The one exception is the ARP-table clear in network repair, which requires an elevated run; without elevation it is reported as failed rather than as success.

## Known limitations

- IPv4 only: the ICMP pinger uses `IcmpSendEcho` and no code path handles IPv6 targets.
- Windows only: the engine links Windows system APIs and has no portable backend.

## License

MIT — see [LICENSE](LICENSE).

## Contributing

File issues in the repository tracker with the steps to reproduce and, for probe-related bugs, the relevant entry from the diagnostics panel. Fork the repository and open a pull request against `main` with a focused, single-purpose change. Before submitting, make sure `cargo fmt`, `cargo clippy`, and `cargo test` pass in `src-tauri/`, and `npm run build` passes at the repository root.
