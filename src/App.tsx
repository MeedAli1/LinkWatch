import { playPacketLossChime, ChimeProfile } from './audioChimes';
import React, { useState, useEffect, useCallback, useRef } from 'react';
import appLogo from './assets/logo.png';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  Activity, AlertTriangle, CheckCircle2, Play, Square,
  Download, FileText, ShieldCheck, Server, Wifi, Globe,
  Radio, Sun, Moon, Plus, Trash2, ToggleLeft, ToggleRight,
  PlusCircle, X, Wrench, Volume2, Network, Navigation, Compass, Settings, ArrowDown, ArrowUp, Gamepad2, Users, LifeBuoy, Database, HardDrive, RefreshCw, BarChart2, TrendingUp, Zap, FileSpreadsheet, Layers, Check, Keyboard
} from 'lucide-react';

// ─── Types ──────────────────────────────────────────────────────────────────

interface ProbeObservation {
  target_id: string;
  target_label: string;
  host: string;
  resolved_ip: string | null;
  timestamp_ms: number;
  rtt_api_ms: number | null;
  rtt_measured_us: number | null;
  success: boolean;
  status_code: number;
  error_msg: string | null;
}

interface TargetStat {
  target_id: string;
  target_label: string;
  host: string;
  total_probes: number;
  success_count: number;
  loss_percent: number;
  avg_rtt_ms: number;
  min_rtt_ms: number;
  max_rtt_ms: number;
  last_rtt_ms: number | null;
  last_status: boolean;
  last_seen_ms: number;
}

interface DiagnosticFinding {
  status: 'Healthy' | 'Degraded' | 'LocalIssue' | 'GatewayIssue' | 'IspOutage' | 'TargetSpecific' | 'Unknown';
  culprit: 'None' | 'LocalWifi' | 'GatewayRouter' | 'IspWan' | 'RemoteTarget';
  title: string;
  observation: string;
  hypothesis: string;
  confidence: 'None' | 'Weak' | 'Moderate' | 'Strong';
  recommendation: string;
  affected_scope: string;
  quality_score: number;
  quality_grade: string;
  jitter_ms: number;
}

interface OutageEvent {
  id: number;
  start_ms: number;
  end_ms: number | null;
  duration_seconds: number | null;
  affected_targets: string;
  summary: string;
}

interface CustomTarget {
  id: number;
  target_id: string;
  label: string;
  host: string;
  enabled: boolean;
  created_ms: number;
}

interface AppStateSnapshot {
  is_monitoring: boolean;
  storage_degraded: boolean;
  active_gateway: string | null;
  latest_observations: ProbeObservation[];
  stats: TargetStat[];
  diagnostic: DiagnosticFinding;
  recent_outages: OutageEvent[];
  custom_targets: CustomTarget[];
}

interface RetentionSettings {
  pruning_enabled: boolean;
  max_age_days: number;
  max_records: number;
  last_cleanup_ms: number | null;
}

interface DbStats {
  probe_count: number;
  outage_count: number;
  custom_target_count: number;
  db_size_bytes: number;
  wal_size_bytes: number;
  last_cleanup_ms: number | null;
  oldest_probe_ms: number | null;
}

interface TimeSeriesBucket {
  bucket_start_ms: number;
  bucket_end_ms: number;
  total_probes: number;
  success_count: number;
  loss_count: number;
  loss_percent: number;
  avg_rtt_ms: number | null;
  min_rtt_ms: number | null;
  max_rtt_ms: number | null;
  last_resolved_ip: string | null;
}

interface TargetTimeSeriesResult {
  target_id: string;
  start_ms: number;
  end_ms: number;
  bucket_size_ms: number;
  buckets: TimeSeriesBucket[];
  recent_probes: ProbeObservation[];
  outages: OutageEvent[];
}

export interface BufferbloatResult {
  baseline_ping_ms: number;
  loaded_ping_ms: number;
  bufferbloat_delta_ms: number;
  grade: string;
  grade_summary: string;
  download_mbps: number;
  tested_at_ms: number;
}

export type OverlayCorner = 'top_left' | 'top_right' | 'bottom_left' | 'bottom_right';

export interface OverlayConfig {
  corner: OverlayCorner;
  font_size: number;
  color: string;
  opacity: number;
  margin: number;
}

interface AutostartConfig {
  autostart_enabled: boolean;
  start_minimized: boolean;
}

// ─── Hotkey Types ────────────────────────────────────────────────────────────

type HotkeyAction = 'toggle_window' | 'toggle_mini_hud' | 'network_repair' | 'open_diagnostics';

interface HotkeyBinding {
  action: HotkeyAction;
  accelerator: string | null;
  enabled: boolean;
}

interface HotkeyConfig {
  bindings: HotkeyBinding[];
}

const HOTKEY_ACTION_LABELS: Record<HotkeyAction, { label: string; desc: string }> = {
  toggle_window:    { label: 'Show / Hide Window',       desc: 'Toggle the main LinkWatch window from anywhere' },
  toggle_mini_hud:  { label: 'Toggle Mini HUD',          desc: 'Show or hide the floating always-on-top PiP widget' },
  network_repair:   { label: '1-Click Network Repair',   desc: 'Instantly flush DNS & reset network stack' },
  open_diagnostics: { label: 'Open Diagnostics Panel',   desc: 'Jump straight to the fault-analysis view' },
};

interface DnsResolverResult {
  id: string;
  name: string;
  server_ip: string;
  avg_latency_ms: number;
  min_latency_ms: number;
  max_latency_ms: number;
  success_count: number;
  query_count: number;
  is_fastest: boolean;
}

interface DnsBenchmarkResult {
  resolvers: DnsResolverResult[];
  fastest_resolver_name: string;
  recommendation: string;
  tested_at_ms: number;
}

interface WifiDetails {
  is_connected: boolean;
  ssid: string;
  bssid: string;
  signal_percent: number;
  channel: number;
  band: string;
  link_speed_mbps: number;
  interface_name: string;
  security: string;
  health_verdict: string;
}

interface WifiNeighbor {
  ssid: string;
  bssid: string;
  signal_percent: number;
  channel: number;
  band: string;
  security: string;
  is_current: boolean;
}

interface ChannelCongestion {
  channel: number;
  band: string;
  network_count: number;
  strongest_signal: number;
  is_your_channel: boolean;
}

interface WifiEnvironmentScan {
  your_network: WifiDetails;
  neighbors: WifiNeighbor[];
  channel_congestion: ChannelCongestion[];
  interference_verdict: string;
}

interface TracerouteHop {
  hop_number: number;
  ip: string | null;
  hostname: string | null;
  rtt_ms: number | null;
  status: string;
  is_destination: boolean;
}

interface TracerouteResult {
  target: string;
  destination_ip: string;
  hops: TracerouteHop[];
  total_hops: number;
  completed: boolean;
}

interface ThroughputData {
  download_kbps: number;
  upload_kbps: number;
  download_mbps: number;
  upload_mbps: number;
  interface_name: string;
  total_rx_bytes: number;
  total_tx_bytes: number;
}

interface GameRegionPing {
  game_title: string;
  region_name: string;
  endpoint_host: string;
  ping_ms: number | null;
  quality_tier: string;
}

interface GameRadarResult {
  pings: GameRegionPing[];
  lowest_ping_region: string;
  lowest_ping_ms: number;
  scanned_at_ms: number;
}

interface LanDevice {
  ip: string;
  mac: string;
  vendor: string;
  device_type: string;
  hostname: string;
  is_gateway: boolean;
  is_current_host: boolean;
  ping_ms: number | null;
}

interface LanScanResult {
  subnet: string;
  devices: LanDevice[];
  total_devices_found: number;
  scanned_at_ms: number;
}

interface RepairStepResult {
  step_name: string;
  command: string;
  success: boolean;
  output: string;
}

interface NetworkRepairResult {
  steps: RepairStepResult[];
  overall_success: boolean;
  summary: string;
  timestamp_ms: number;
}

// ─── App ─────────────────────────────────────────────────────────────────────

type TabId = 'live' | 'stats' | 'outages' | 'targets' | 'tools' | 'settings';

export function App() {
  const [snapshot, setSnapshot] = useState<AppStateSnapshot | null>(null);
  useEffect(() => {
    fetchThroughput();
    const interval = setInterval(fetchThroughput, 2000);
    return () => clearInterval(interval);
  }, []);
  const [isDark, setIsDark] = useState<boolean>(
    () => localStorage.getItem('linkwatch_theme') !== 'light',
  );
  useEffect(() => {
    document.documentElement.dataset.theme = isDark ? 'dark' : 'light';
    localStorage.setItem('linkwatch_theme', isDark ? 'dark' : 'light');
  }, [isDark]);
  const [activeTab, setActiveTab] = useState<TabId>('live');
  const [notice, setNotice] = useState<{ msg: string; kind: 'ok' | 'err' } | null>(null);

  // Add-target form state
  const [addLabel, setAddLabel] = useState('');
  const [addHost, setAddHost] = useState('');
  const [addError, setAddError] = useState<string | null>(null);
  const [addLoading, setAddLoading] = useState(false);
  // Retention & DB settings state
  const [dbStats, setDbStats] = useState<DbStats | null>(null);
  const [retentionSettings, setRetentionSettings] = useState<RetentionSettings>({
    pruning_enabled: true,
    max_age_days: 7,
    max_records: 50000,
    last_cleanup_ms: null,
  });
  const [cleaningUp, setCleaningUp] = useState(false);
  const [savingSettings, setSavingSettings] = useState(false);
  // Historical inspection state
  const [selectedTargetId, setSelectedTargetId] = useState<string>('gateway');
  const [timeWindowMin, setTimeWindowMin] = useState<number>(60); // 15, 60, 360, 1440
  const [timeSeriesData, setTimeSeriesData] = useState<TargetTimeSeriesResult | null>(null);
  const [loadingTimeSeries, setLoadingTimeSeries] = useState(false);
  // Autostart & Window behavior state
  const [autostartConfig, setAutostartConfig] = useState<AutostartConfig>({
    autostart_enabled: false,
    start_minimized: false,
  });
  const [savingAutostart, setSavingAutostart] = useState(false);

  // ─── Probe interval state ─────────────────────────────────────────────────
  const [probeIntervalMs, setProbeIntervalMs] = useState<number>(1500);
  const [savingInterval, setSavingInterval] = useState(false);

  // ─── Hotkey state ──────────────────────────────────────────────────────────
  const [hotkeyConfig, setHotkeyConfig] = useState<HotkeyConfig>({ bindings: [] });
  const [savingHotkeys, setSavingHotkeys] = useState(false);
  const [recordingAction, setRecordingAction] = useState<HotkeyAction | null>(null);
  const recordingRef = useRef<HotkeyAction | null>(null);
  // Tools Tab state: DNS benchmark, Wi-Fi inspector, Traceroute, Audio Chimes
  const [dnsBenchResult, setDnsBenchResult] = useState<DnsBenchmarkResult | null>(null);
  const [runningDnsBench, setRunningDnsBench] = useState(false);

  const [wifiDetails, setWifiDetails] = useState<WifiDetails | null>(null);
  const [wifiScan, setWifiScan] = useState<WifiEnvironmentScan | null>(null);
  const [fetchingWifi, setFetchingWifi] = useState(false);

  const [tracerouteTarget, setTracerouteTarget] = useState('1.1.1.1');
  const [tracerouteResult, setTracerouteResult] = useState<TracerouteResult | null>(null);
  const [runningTraceroute, setRunningTraceroute] = useState(false);

  // Real-Time Throughput state (Feature A)
  const [throughput, setThroughput] = useState<ThroughputData | null>(null);

  // Bufferbloat test state
  const [bufferbloatResult, setBufferbloatResult] = useState<BufferbloatResult | null>(null);
  const [runningBufferbloat, setRunningBufferbloat] = useState(false);

  // Game Ping Radar state (Feature B)
  const [gameRadarResult, setGameRadarResult] = useState<GameRadarResult | null>(null);
  const [scanningGameRadar, setScanningGameRadar] = useState(false);

  // Local LAN Scanner state (Feature C)
  const [lanScanResult, setLanScanResult] = useState<LanScanResult | null>(null);
  const [scanningLan, setScanningLan] = useState(false);
  // 1-Click Network Repair state
  const [repairResult, setRepairResult] = useState<NetworkRepairResult | null>(null);
  const [runningRepair, setRunningRepair] = useState(false);
  // Overlay HUD state
  const [overlayOpen, setOverlayOpen] = useState(false);
  // Chime settings
  const [chimeProfile, setChimeProfile] = useState<ChimeProfile>(() => {
    return (localStorage.getItem('linkwatch_chime_profile') as ChimeProfile) || 'descending_triad';
  });
  const [chimeEnabled, setChimeEnabled] = useState<boolean>(() => {
    return localStorage.getItem('linkwatch_chime_enabled') === 'true';
  });

  const handleSetChimeProfile = (p: ChimeProfile) => {
    setChimeProfile(p);
    localStorage.setItem('linkwatch_chime_profile', p);
    playPacketLossChime(p, 0.4);
  };

  const handleToggleChime = () => {
    setChimeEnabled(prev => {
      const next = !prev;
      localStorage.setItem('linkwatch_chime_enabled', String(next));
      if (next) playPacketLossChime(chimeProfile, 0.4);
      return next;
    });
  };

  const [overlayConfig, setOverlayConfig] = useState<OverlayConfig>({
    corner: 'top_right',
    font_size: 16,
    color: '#06b6d4',
    opacity: 0.95,
    margin: 20,
  });
  const [savingOverlay, setSavingOverlay] = useState(false);

  const showNotice = (msg: string, kind: 'ok' | 'err' = 'ok') => {
    setNotice({ msg, kind });
    setTimeout(() => setNotice(null), 3500);
  };

  const fetchSnapshot = useCallback(async () => {
    try {
      const data = await invoke<AppStateSnapshot>('get_snapshot');
      setSnapshot(data);
    } catch (err) {
      console.error('snapshot error:', err);
    }
  }, []);

  // Adaptive background visibility polling: throttles when LinkWatch is minimized to tray
  useEffect(() => {
    fetchSnapshot();

    let id: number;
    const scheduleNext = () => {
      // 1.5s when visible in foreground, 5.0s when window is hidden/minimized to tray
      const intervalMs = document.hidden ? 5000 : 1500;
      id = window.setTimeout(async () => {
        await fetchSnapshot();
        scheduleNext();
      }, intervalMs);
    };

    scheduleNext();

    const handleVisibilityChange = () => {
      if (!document.hidden) {
        fetchSnapshot();
      }
    };

    document.addEventListener('visibilitychange', handleVisibilityChange);
    return () => {
      clearTimeout(id);
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, [fetchSnapshot]);

  // History recording is failing while the charts keep moving from the
  // in-memory ring; tell the user once per transition instead of per poll.
  const storageDegradedNoticed = useRef(false);
  useEffect(() => {
    if (!snapshot) return;
    if (snapshot.storage_degraded && !storageDegradedNoticed.current) {
      storageDegradedNoticed.current = true;
      showNotice('History is not being saved - check free disk space', 'err');
    } else if (!snapshot.storage_degraded) {
      storageDegradedNoticed.current = false;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snapshot?.storage_degraded]);

  const toggleMonitoring = async () => {
    if (!snapshot) return;
    try {
      await invoke(snapshot.is_monitoring ? 'stop_monitoring' : 'start_monitoring');
      await fetchSnapshot();
    } catch (e) { console.error(e); }
  };

  const handleExportCSV = async () => {
    try {
      const csv = await invoke<string>('export_csv_data');
      downloadBlob(csv, 'text/csv', `linkwatch_${Date.now()}.csv`);
      showNotice('CSV exported');
    } catch (e) { showNotice(String(e), 'err'); }
  };

  const handleExportHTML = async () => {
    try {
      const html = await invoke<string>('export_html_report_data');
      downloadBlob(html, 'text/html', `linkwatch_report_${Date.now()}.html`);
      showNotice('HTML report generated');
    } catch (e) { showNotice(String(e), 'err'); }
  };
  const handleExportISPReport = async () => {
    try {
      const html = await invoke<string>('export_isp_dispute_report');
      downloadBlob(html, 'text/html', `linkwatch_isp_dispute_${Date.now()}.html`);
      showNotice('ISP Dispute Report generated');
    } catch (e) { showNotice(String(e), 'err'); }
  };

  const handleAddTarget = async () => {
    setAddError(null);
    if (!addLabel.trim() || !addHost.trim()) {
      setAddError('Both name and address are required.');
      return;
    }
    setAddLoading(true);
    try {
      await invoke('add_custom_target', { label: addLabel.trim(), host: addHost.trim() });
      setAddLabel('');
      setAddHost('');
      await fetchSnapshot();
      showNotice('Target added and monitoring started');
    } catch (e) {
      setAddError(String(e));
    } finally {
      setAddLoading(false);
    }
  };

  const handleToggleEnabled = async (t: CustomTarget) => {
    try {
      await invoke('set_custom_target_enabled', { targetId: t.target_id, enabled: !t.enabled });
      await fetchSnapshot();
      showNotice(t.enabled ? `Paused monitoring for "${t.label}"` : `Resumed monitoring for "${t.label}"`);
    } catch (e) { showNotice(String(e), 'err'); }
  };

  const handleRemove = async (t: CustomTarget) => {
    try {
      await invoke('remove_custom_target', { targetId: t.target_id });
      await fetchSnapshot();
      showNotice(`Removed "${t.label}" - historical data preserved`);
    } catch (e) { showNotice(String(e), 'err'); }
  };

  const fetchDbStats = async () => {
    try {
      const stats = await invoke<DbStats>('get_db_stats');
      setDbStats(stats);
    } catch (err) {
      console.error('get_db_stats error:', err);
    }
  };

  const fetchRetentionSettings = async () => {
    try {
      const settings = await invoke<RetentionSettings>('get_retention_settings');
      setRetentionSettings(settings);
    } catch (err) {
      console.error('get_retention_settings error:', err);
    }
  };

  const handleSaveRetention = async () => {
    setSavingSettings(true);
    try {
      await invoke('save_retention_settings', { settings: retentionSettings });
      showNotice('Retention settings saved');
      await fetchDbStats();
    } catch (err) {
      showNotice(String(err), 'err');
    } finally {
      setSavingSettings(false);
    }
  };

  const handleRunCleanup = async () => {
    setCleaningUp(true);
    try {
      const stats = await invoke<DbStats>('run_cleanup_now');
      setDbStats(stats);
      showNotice('Database cleanup completed');
      await fetchRetentionSettings();
      fetchAutostartConfig();
    } catch (err) {
      showNotice(String(err), 'err');
    } finally {
      setCleaningUp(false);
    }
  };
  const fetchAutostartConfig = async () => {
    try {
      const config = await invoke<AutostartConfig>('get_autostart_config');
      setAutostartConfig(config);
    } catch (err) {
      console.error('get_autostart_config error:', err);
    }
  };

  // ─── Probe interval logic ─────────────────────────────────────────────────

  const fetchProbeInterval = async () => {
    try {
      const ms = await invoke<number>('get_probe_interval');
      setProbeIntervalMs(ms);
    } catch (err) {
      console.error('get_probe_interval error:', err);
    }
  };

  const handleSaveProbeInterval = async () => {
    setSavingInterval(true);
    try {
      await invoke('set_probe_interval', { intervalMs: probeIntervalMs });
      showNotice(`Probe interval set to ${probeIntervalMs} ms`);
    } catch (err) {
      showNotice(String(err), 'err');
    } finally {
      setSavingInterval(false);
    }
  };

  // ─── Hotkey logic ─────────────────────────────────────────────────────────

  const fetchHotkeyConfig = useCallback(async () => {
    try {
      const cfg = await invoke<HotkeyConfig>('get_hotkey_config');
      setHotkeyConfig(cfg);
    } catch (err) {
      console.error('get_hotkey_config error:', err);
    }
  }, []);

  const handleSaveHotkeys = async (cfg: HotkeyConfig) => {
    setSavingHotkeys(true);
    try {
      await invoke('save_hotkey_config_cmd', { config: cfg });
      showNotice('Global hotkeys saved & registered');
    } catch (err) {
      showNotice(String(err), 'err');
    } finally {
      setSavingHotkeys(false);
    }
  };

  const toggleHotkeyEnabled = (action: HotkeyAction) => {
    const updated: HotkeyConfig = {
      bindings: hotkeyConfig.bindings.map(b =>
        b.action === action ? { ...b, enabled: !b.enabled } : b
      ),
    };
    setHotkeyConfig(updated);
  };

  const clearHotkey = (action: HotkeyAction) => {
    const updated: HotkeyConfig = {
      bindings: hotkeyConfig.bindings.map(b =>
        b.action === action ? { ...b, accelerator: null, enabled: false } : b
      ),
    };
    setHotkeyConfig(updated);
  };

  // Key capture: when recording, intercept the next keydown combo
  const startRecording = (action: HotkeyAction) => {
    setRecordingAction(action);
    recordingRef.current = action;
  };

  const handleRecordKeyDown = useCallback((e: KeyboardEvent) => {
    const action = recordingRef.current;
    if (!action) return;

    // Ignore pure modifier presses
    if (['Control', 'Shift', 'Alt', 'Meta', 'CapsLock', 'Tab', 'Escape'].includes(e.key)) {
      if (e.key === 'Escape') {
        setRecordingAction(null);
        recordingRef.current = null;
      }
      return;
    }

    e.preventDefault();
    e.stopPropagation();

    // Build accelerator string
    const parts: string[] = [];
    if (e.ctrlKey)  parts.push('Ctrl');
    if (e.shiftKey) parts.push('Shift');
    if (e.altKey)   parts.push('Alt');
    // Normalize key name
    const keyName = e.key.length === 1 ? e.key.toUpperCase() : e.key;
    parts.push(keyName);
    const accel = parts.join('+');

    setHotkeyConfig(prev => ({
      bindings: prev.bindings.map(b =>
        b.action === action ? { ...b, accelerator: accel, enabled: true } : b
      ),
    }));
    setRecordingAction(null);
    recordingRef.current = null;
  }, []);

  useEffect(() => {
    if (recordingAction) {
      window.addEventListener('keydown', handleRecordKeyDown, true);
    } else {
      window.removeEventListener('keydown', handleRecordKeyDown, true);
    }
    return () => window.removeEventListener('keydown', handleRecordKeyDown, true);
  }, [recordingAction, handleRecordKeyDown]);

  const handleToggleAutostart = async (key: 'autostart_enabled' | 'start_minimized') => {
    const updated = {
      ...autostartConfig,
      [key]: !autostartConfig[key],
    };
    setAutostartConfig(updated);
    setSavingAutostart(true);
    try {
      await invoke('set_autostart_config', {
        autostartEnabled: updated.autostart_enabled,
        startMinimized: updated.start_minimized,
      });
      showNotice(
        key === 'autostart_enabled'
          ? updated.autostart_enabled
            ? 'Windows autostart enabled'
            : 'Windows autostart disabled'
          : updated.start_minimized
            ? 'Start minimized to tray enabled'
            : 'Start minimized to tray disabled'
      );
    } catch (err) {
      showNotice(String(err), 'err');
      await fetchAutostartConfig();
    } finally {
      setSavingAutostart(false);
    }
  };
  const handleRunDnsBenchmark = async () => {
    setRunningDnsBench(true);
    try {
      const res = await invoke<DnsBenchmarkResult>('run_dns_benchmark');
      setDnsBenchResult(res);
      showNotice(`DNS Benchmark complete: ${res.fastest_resolver_name} is fastest`);
    } catch (err) {
      showNotice(String(err), 'err');
    } finally {
      setRunningDnsBench(false);
    }
  };

  const handleFetchWifi = async () => {
    setFetchingWifi(true);
    try {
      const scanRes = await invoke<WifiEnvironmentScan>('scan_wifi_environment');
      setWifiScan(scanRes);
      setWifiDetails(scanRes.your_network);
    } catch {
      // Fallback to basic wifi details if environment scan fails
      try {
        const res = await invoke<WifiDetails>('get_wifi_details');
        setWifiDetails(res);
      } catch (err) {
        showNotice(String(err), 'err');
      }
    } finally {
      setFetchingWifi(false);
    }
  };

  const handleRunTraceroute = async () => {
    if (!tracerouteTarget.trim()) return;
    setRunningTraceroute(true);
    try {
      const res = await invoke<TracerouteResult>('run_traceroute', {
        target: tracerouteTarget.trim(),
        maxHops: 20,
      });
      setTracerouteResult(res);
      showNotice(`Traceroute mapped ${res.total_hops} hops`);
    } catch (err) {
      showNotice(String(err), 'err');
    } finally {
      setRunningTraceroute(false);
    }
  };

  const fetchThroughput = async () => {
    try {
      const data = await invoke<ThroughputData>('get_adapter_throughput');
      setThroughput(data);
    } catch (err) {
      console.error('get_adapter_throughput error:', err);
    }
  };

  const handleRunBufferbloat = async () => {
    setRunningBufferbloat(true);
    try {
      const res = await invoke<BufferbloatResult>('run_bufferbloat_test');
      setBufferbloatResult(res);
      showNotice(`Bufferbloat test complete: Grade ${res.grade} (+${res.bufferbloat_delta_ms}ms loaded bloat)`);
    } catch (err) {
      showNotice(String(err), 'err');
    } finally {
      setRunningBufferbloat(false);
    }
  };

  const handleRunGameRadar = async () => {
    setScanningGameRadar(true);
    try {
      const res = await invoke<GameRadarResult>('run_game_radar');
      setGameRadarResult(res);
      showNotice(`Game radar scan complete: Lowest ping is ${res.lowest_ping_ms.toFixed(0)}ms (${res.lowest_ping_region})`);
    } catch (err) {
      showNotice(String(err), 'err');
    } finally {
      setScanningGameRadar(false);
    }
  };

  const handleScanLan = async () => {
    setScanningLan(true);
    try {
      const res = await invoke<LanScanResult>('scan_local_lan');
      setLanScanResult(res);
      showNotice(`Discovered ${res.total_devices_found} devices on local LAN`);
    } catch (err) {
      showNotice(String(err), 'err');
    } finally {
      setScanningLan(false);
    }
  };
  const handleExecuteRepair = async () => {
    setRunningRepair(true);
    try {
      const res = await invoke<NetworkRepairResult>('execute_network_repair');
      setRepairResult(res);
      showNotice(res.overall_success ? 'Network stack repair completed successfully' : 'Network repair completed with warnings');
      await fetchSnapshot();
    } catch (err) {
      showNotice(String(err), 'err');
    } finally {
      setRunningRepair(false);
    }
  };
  const fetchOverlayConfig = async () => {
    try {
      const cfg = await invoke<OverlayConfig>('get_overlay_config');
      setOverlayConfig(cfg);
    } catch (e) {
      console.error('get_overlay_config error:', e);
    }
  };

  const handleSaveOverlayConfig = async (nextCfg: OverlayConfig) => {
    setSavingOverlay(true);
    try {
      await invoke('save_overlay_config_cmd', { config: nextCfg });
      setOverlayConfig(nextCfg);
      showNotice('Overlay settings saved & applied', 'ok');
    } catch (e) {
      showNotice(`Failed to save overlay: ${e}`, 'err');
    } finally {
      setSavingOverlay(false);
    }
  };

  const handleToggleOverlay = async () => {
    try {
      if (overlayOpen) {
        await invoke('close_overlay');
        setOverlayOpen(false);
      } else {
        await invoke('open_overlay');
        setOverlayOpen(true);
      }
    } catch (err) {
      showNotice(String(err), 'err');
    }
  };

  // ─── Tauri event listeners for global hotkey actions ─────────────────────
  useEffect(() => {
    const unlisteners: (() => void)[] = [];

    // Backend owns the HUD toggle; sync our button state from its event.
    // (The old 'hotkey-toggle-overlay' listener here was never emitted and
    // guessed the direction by inverting local state, leaving it stale.)
    listen<boolean>('overlay-visibility-changed', (e) => {
      setOverlayOpen(e.payload);
    }).then(u => unlisteners.push(u));

    listen('hotkey-network-repair', () => {
      setActiveTab('tools' as any);
      setRunningRepair(true);
    }).then(u => unlisteners.push(u));

    listen<NetworkRepairResult>('network-repair-result', (e) => {
      setRepairResult(e.payload);
      setRunningRepair(false);
      showNotice(e.payload.overall_success ? 'Network stack repair completed successfully' : 'Network repair completed with warnings');
      void fetchSnapshot();
    }).then(u => unlisteners.push(u));

    listen<string>('hotkey-repair-failed', (e) => {
      setRunningRepair(false);
      showNotice(`Network repair failed: ${e.payload}`, 'err');
    }).then(u => unlisteners.push(u));

    listen('hotkey-open-diagnostics', () => {
      setActiveTab('live' as any);
    }).then(u => unlisteners.push(u));

    return () => unlisteners.forEach(u => u());
  }, []);
  const fetchTimeSeries = async () => {
    if (!selectedTargetId) return;
    setLoadingTimeSeries(true);
    try {
      const now = Date.now();
      const startMs = now - (timeWindowMin * 60 * 1000);
      // Determine bucket size based on window
      // 15m -> 15s (60 buckets), 1h -> 60s (60 buckets), 6h -> 360s (60 buckets), 24h -> 1440s (60 buckets)
      const bucketSizeMs = Math.max(15_000, Math.floor((timeWindowMin * 60 * 1000) / 60));
      const result = await invoke<TargetTimeSeriesResult>('get_target_timeseries', {
        targetId: selectedTargetId,
        startMs,
        endMs: now,
        bucketSizeMs,
      });
      setTimeSeriesData(result);
    } catch (err) {
      console.error('get_target_timeseries error:', err);
    } finally {
      setLoadingTimeSeries(false);
    }
  };

  useEffect(() => {
    if (activeTab === 'stats' && selectedTargetId) {
      fetchTimeSeries();
      const interval = setInterval(fetchTimeSeries, 5000);
      return () => clearInterval(interval);
    }
  }, [activeTab, selectedTargetId, timeWindowMin]);

    useEffect(() => {
    if (activeTab === 'tools') {
      handleFetchWifi();
    }
  }, [activeTab]);

  useEffect(() => {
    if (activeTab === 'settings') {
      fetchDbStats();
      fetchRetentionSettings();
      fetchAutostartConfig();
      fetchHotkeyConfig();
      fetchOverlayConfig();
      fetchProbeInterval();
      const interval = setInterval(fetchDbStats, 5000);
      return () => clearInterval(interval);
    }
  }, [activeTab]);

  const bg = 'bg-[var(--bg-canvas)] text-[var(--text-primary)]';
  const headerBg = 'border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)]/95 backdrop-blur-sm';
  const tabActive = 'bg-[var(--bg-panel-hover)] text-[var(--text-primary)] border-b-2 border-sky-400 font-medium';
  const tabInactive = 'text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-panel)] border-b-2 border-transparent';

  return (
    <div className={`min-h-screen ${bg} flex flex-col font-sans transition-colors duration-150`}>
      {/* Precision Windows Header (48px) */}
      <header className={`h-12 border-b ${headerBg} px-4 flex items-center justify-between sticky top-0 z-20 select-none`}>
        <div className="flex items-center space-x-2.5">
          <img src={appLogo} alt="LinkWatch Logo" className="h-7 w-7 rounded-md border border-[var(--border-hairline)] object-contain shadow-sm shrink-0" />
          <div className="flex items-center space-x-2">
            <span className="text-xs font-bold tracking-tight text-[var(--text-primary)]">LinkWatch</span>
            <span className="text-[10px] font-mono px-1.5 py-0.2 rounded border border-[var(--border-hairline)] bg-[var(--bg-panel-hover)] text-[var(--text-muted)]">v0.1.0</span>
          </div>
        </div>

        {/* Live Bandwidth Speedometer */}
        {throughput && (
          <div className="hidden md:flex items-center space-x-2.5 px-2.5 py-1 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] text-[11px] font-mono tabular-nums">
            <div className="flex items-center space-x-1 text-[var(--text-primary)]">
              <ArrowDown className="h-3 w-3 text-sky-400" />
              <span>
                {throughput.download_mbps >= 1.0
                  ? `${throughput.download_mbps.toFixed(1)} MB/s`
                  : `${throughput.download_kbps.toFixed(0)} KB/s`}
              </span>
            </div>
            <span className="text-[var(--border-subtle)]">|</span>
            <div className="flex items-center space-x-1 text-[var(--text-primary)]">
              <ArrowUp className="h-3 w-3 text-emerald-400" />
              <span>
                {throughput.upload_mbps >= 1.0
                  ? `${throughput.upload_mbps.toFixed(1)} MB/s`
                  : `${throughput.upload_kbps.toFixed(0)} KB/s`}
              </span>
            </div>
          </div>
        )}

        <div className="flex items-center space-x-1.5">
          {notice && (
            <span className={`text-[11px] px-2 py-0.5 rounded-md border flex items-center space-x-1.5 ${
              notice.kind === 'err'
                ? 'bg-rose-500/10 text-rose-300 border-rose-500/30'
                : 'bg-emerald-500/10 text-emerald-300 border-emerald-500/30'
            }`}>
              <span>{notice.msg}</span>
              <X className="h-3 w-3 cursor-pointer opacity-70 hover:opacity-100" onClick={() => setNotice(null)} />
            </span>
          )}

          <button onClick={toggleMonitoring}
            className={`flex items-center space-x-1.5 px-2.5 py-1 rounded-md text-xs font-medium border transition-colors ${
              snapshot?.is_monitoring
                ? 'bg-rose-500/15 hover:bg-rose-500/25 text-rose-300 border-rose-500/30'
                : 'bg-emerald-500/15 hover:bg-emerald-500/25 text-emerald-300 border-emerald-500/30'
            }`}>
            {snapshot?.is_monitoring
              ? <><Square className="h-3 w-3 fill-current" /><span>Pause</span></>
              : <><Play className="h-3 w-3 fill-current" /><span>Resume</span></>}
          </button>

          <button onClick={handleExportCSV} title="Export CSV"
            className="p-1.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] hover:bg-[var(--bg-panel-hover)] text-xs text-[var(--text-muted)] hover:text-[var(--text-primary)] flex items-center space-x-1 transition-colors">
            <Download className="h-3.5 w-3.5" /><span className="hidden sm:inline">CSV</span>
          </button>

          <button onClick={handleExportHTML} title="Diagnostic Report"
            className="p-1.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] hover:bg-[var(--bg-panel-hover)] text-xs text-[var(--text-muted)] hover:text-[var(--text-primary)] flex items-center space-x-1 transition-colors">
            <FileText className="h-3.5 w-3.5" /><span className="hidden sm:inline">Report</span>
          </button>

          <button onClick={handleExportISPReport} title="ISP SLA Dispute Notice"
            className="p-1.5 rounded-md border border-amber-500/30 bg-amber-500/10 hover:bg-amber-500/20 text-xs text-amber-300 flex items-center space-x-1 transition-colors">
            <FileSpreadsheet className="h-3.5 w-3.5 text-amber-400" /><span className="hidden sm:inline font-medium">ISP Dispute</span>
          </button>

          <button onClick={handleExecuteRepair} disabled={runningRepair} title="Network Stack Reset"
            className="p-1.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] hover:bg-[var(--bg-panel-hover)] text-xs text-[var(--text-primary)] flex items-center space-x-1.5 disabled:opacity-50 transition-colors">
            <LifeBuoy className={`h-3.5 w-3.5 text-sky-400 ${runningRepair ? 'animate-spin' : ''}`} />
            <span className="hidden sm:inline font-medium">Fix Stack</span>
          </button>

          <button
            onClick={() => handleToggleOverlay()}
            title={overlayOpen ? 'Close Overlay HUD' : 'Open Transparent Overlay HUD'}
            className={`p-1.5 rounded-md border transition-colors ${
              overlayOpen
                ? 'border-sky-500/40 bg-sky-500/15 text-sky-300'
                : 'border-[var(--border-hairline)] bg-[var(--bg-panel)] hover:bg-[var(--bg-panel-hover)] text-[var(--text-muted)] hover:text-[var(--text-primary)]'
            }`}
          >
            <Layers className="h-3.5 w-3.5" />
          </button>

          <button onClick={() => setIsDark(!isDark)}
            className="p-1.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] hover:bg-[var(--bg-panel-hover)] text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-colors">
            {isDark ? <Sun className="h-3.5 w-3.5" /> : <Moon className="h-3.5 w-3.5" />}
          </button>
        </div>
      </header>

      {/* Main Container */}
      <main className="flex-1 p-4 space-y-3 max-w-7xl w-full mx-auto">

        {/* Compact Single-Row Diagnostic Status Strip */}
        {snapshot?.diagnostic && (
          <div className="rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] divide-y divide-[var(--border-hairline)] overflow-hidden">
            {/* Primary Scannable Strip */}
            <div className="px-3.5 py-2 flex flex-wrap items-center justify-between gap-2.5 text-xs">
              {/* Left: Health Indicator & Diagnostic Title */}
              <div className="flex items-center space-x-2.5 min-w-0">
                <span className={`inline-block h-2.5 w-2.5 rounded-full shrink-0 ${
                  snapshot.diagnostic.status === 'Healthy'
                    ? 'bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.4)]'
                    : snapshot.diagnostic.status === 'Degraded'
                      ? 'bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.4)]'
                      : 'bg-rose-500 shadow-[0_0_8px_rgba(239,68,68,0.4)]'
                }`} />
                <span className="font-bold text-[var(--text-primary)] tracking-tight truncate">
                  {snapshot.diagnostic.title}
                </span>
                <span className="text-[10px] font-mono px-1.5 py-0.2 rounded border border-[var(--border-hairline)] bg-[var(--bg-panel-hover)] text-[var(--text-muted)] uppercase tracking-wider shrink-0">
                  CONFIDENCE: {snapshot.diagnostic.confidence}
                </span>
                {snapshot.diagnostic.culprit !== 'None' && (
                  <span className="text-[10px] font-mono px-1.5 py-0.2 rounded border border-rose-500/30 bg-rose-500/10 text-rose-300 font-bold uppercase tracking-wider shrink-0">
                    FAULT: {snapshot.diagnostic.affected_scope}
                  </span>
                )}
              </div>

              {/* Center: Clean Connection Path */}
              <div className="hidden lg:flex items-center space-x-1.5 text-[11px] font-mono">
                <span className="text-[var(--text-muted)]">PATH:</span>
                <span className="px-1.5 py-0.5 rounded border border-[var(--border-hairline)] bg-[var(--bg-canvas)] text-[var(--text-primary)]">PC</span>
                <span className="text-[var(--text-dim)]">&rarr;</span>
                <span className={`px-1.5 py-0.5 rounded border ${
                  snapshot.diagnostic.culprit === 'LocalWifi' || snapshot.diagnostic.culprit === 'GatewayRouter'
                    ? 'border-rose-500/40 bg-rose-500/15 text-rose-300 font-bold'
                    : 'border-[var(--border-hairline)] bg-[var(--bg-canvas)] text-[var(--text-primary)]'
                }`}>
                  Gateway
                </span>
                <span className="text-[var(--text-dim)]">&rarr;</span>
                <span className={`px-1.5 py-0.5 rounded border ${
                  snapshot.diagnostic.culprit === 'IspWan'
                    ? 'border-rose-500/40 bg-rose-500/15 text-rose-300 font-bold'
                    : 'border-[var(--border-hairline)] bg-[var(--bg-canvas)] text-[var(--text-primary)]'
                }`}>
                  ISP Uplink
                </span>
                <span className="text-[var(--text-dim)]">&rarr;</span>
                <span className={`px-1.5 py-0.5 rounded border ${
                  snapshot.diagnostic.culprit === 'RemoteTarget'
                    ? 'border-rose-500/40 bg-rose-500/15 text-rose-300 font-bold'
                    : 'border-[var(--border-hairline)] bg-[var(--bg-canvas)] text-[var(--text-primary)]'
                }`}>
                  WAN Target
                </span>
              </div>

              {/* Right: Quality Grade & Jitter Telemetry */}
              <div className="flex items-center space-x-3 text-xs shrink-0 font-mono tabular-nums">
                <div className="flex items-center space-x-1 text-[var(--text-muted)]">
                  <span>GATEWAY:</span>
                  <span className="text-[var(--text-primary)] font-semibold">{snapshot.active_gateway ?? 'Resolving…'}</span>
                </div>
                <span className="text-[var(--border-subtle)]">|</span>
                <div className="flex items-center space-x-1">
                  <span className="text-[var(--text-muted)]">GRADE:</span>
                  <span className={`font-bold px-1 rounded ${
                    snapshot.diagnostic.quality_grade.startsWith('A')
                      ? 'text-emerald-400'
                      : snapshot.diagnostic.quality_grade === 'B'
                        ? 'text-sky-400'
                        : snapshot.diagnostic.quality_grade === 'C'
                          ? 'text-amber-400'
                          : 'text-rose-400'
                  }`}>
                    {snapshot.diagnostic.quality_grade} ({snapshot.diagnostic.quality_score}/100)
                  </span>
                </div>
                <span className="text-[var(--border-subtle)]">|</span>
                <div className="flex items-center space-x-1">
                  <span className="text-[var(--text-muted)]">JITTER:</span>
                  <span className="text-[var(--text-primary)] font-semibold">{snapshot.diagnostic.jitter_ms.toFixed(1)} ms</span>
                </div>
              </div>
            </div>

            {/* Concise 3-Column Diagnostic Analysis Strip */}
            <div className="grid grid-cols-1 md:grid-cols-3 divide-y md:divide-y-0 md:divide-x divide-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-[11px]">
              <div className="px-3.5 py-1.5">
                <span className="font-bold text-[var(--text-muted)] uppercase tracking-wider block text-[10px]">SUMMARY</span>
                <p title={snapshot.diagnostic.observation} className="text-[var(--text-primary)] mt-0.5 leading-relaxed line-clamp-3">{snapshot.diagnostic.observation}</p>
              </div>
              <div className="px-3.5 py-1.5">
                <span className="font-bold text-[var(--text-muted)] uppercase tracking-wider block text-[10px]">ROOT CAUSE</span>
                <p title={snapshot.diagnostic.hypothesis} className="text-[var(--text-primary)] mt-0.5 leading-relaxed line-clamp-3">{snapshot.diagnostic.hypothesis}</p>
              </div>
              <div className="px-3.5 py-1.5">
                <span className="font-bold text-[var(--text-muted)] uppercase tracking-wider block text-[10px]">ACTION</span>
                <p title={snapshot.diagnostic.recommendation} className="text-[var(--text-primary)] mt-0.5 leading-relaxed line-clamp-3">{snapshot.diagnostic.recommendation}</p>
              </div>
            </div>
          </div>
        )}

        {/* Tab Navigation Strip */}
        <div className="flex space-x-1 border-b border-[var(--border-hairline)] pb-1 text-xs">
          {([
            ['live', Activity, 'Live Probes'],
            ['stats', Server, 'Statistics'],
            ['outages', AlertTriangle, `Outages (${snapshot?.recent_outages?.length ?? 0})`],
            ['targets', PlusCircle, `Targets (${snapshot?.custom_targets?.length ?? 0})`],
            ['tools', Wrench, 'Tools & Diagnostics'],
            ['settings', Settings, 'Settings'],
          ] as [TabId, React.ElementType, string][]).map(([id, Icon, label]) => (
            <button key={id} onClick={() => setActiveTab(id)}
              className={`px-3 py-1.5 rounded-t-md flex items-center space-x-1.5 transition-colors ${activeTab === id ? tabActive : tabInactive}`}>
              <Icon className="h-3.5 w-3.5" />
              <span>{label}</span>
            </button>
          ))}
        </div>

        {/* Live Probes Tab */}
        {activeTab === 'live' && (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
            {snapshot?.stats.map((stat) => {
              const latestObs = [...(snapshot.latest_observations ?? [])]
                .reverse().find(o => o.target_id === stat.target_id);
              const rttMs = latestObs?.rtt_measured_us
                ? (latestObs.rtt_measured_us / 1000).toFixed(1) : null;

              return (
                <div key={stat.target_id} className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] hover:border-[var(--border-subtle)] transition-colors space-y-2.5">
                  <div className="flex items-center justify-between">
                    <span className="text-[11px] font-mono text-[var(--text-muted)] flex items-center space-x-1.5 truncate max-w-[140px]">
                      {stat.target_id === 'gateway' ? <Wifi className="h-3 w-3 text-sky-400 shrink-0" /> : <Globe className="h-3 w-3 text-[var(--text-muted)] shrink-0" />}
                      <span className="truncate">{stat.host}</span>
                    </span>
                    <span className={`h-2 w-2 rounded-full shrink-0 ${
                      latestObs?.success
                        ? 'bg-emerald-500 shadow-[0_0_6px_rgba(16,185,129,0.5)]'
                        : 'bg-rose-500 animate-pulse'
                    }`} />
                  </div>
                  <div>
                    <h3 className="text-xs font-bold text-[var(--text-primary)] tracking-tight truncate">{stat.target_label}</h3>
                    <div className="flex items-baseline space-x-1 mt-0.5">
                      <span className="text-2xl font-black font-mono tabular-nums tracking-tight text-[var(--text-primary)]">
                        {latestObs?.success ? rttMs : '—'}
                      </span>
                      <span className="text-[11px] font-mono text-[var(--text-muted)]">ms</span>
                    </div>
                  </div>

                  {/* High-Precision Instrumentation Sparkline */}
                  <div className="pt-1">
                    <LiveSparkline
                      observations={
                        snapshot?.latest_observations?.filter(o => o.target_id === stat.target_id) ?? []
                      }
                    />
                  </div>

                  <div className="pt-2 border-t border-[var(--border-hairline)] grid grid-cols-2 text-[11px] font-mono tabular-nums text-[var(--text-muted)]">
                    <div>
                      LOSS: <span className={`font-semibold ${stat.loss_percent > 0 ? 'text-rose-400' : 'text-emerald-400'}`}>{stat.loss_percent.toFixed(1)}%</span>
                    </div>
                    <div className="text-right">
                      AVG: <span className="font-semibold text-[var(--text-primary)]">{stat.avg_rtt_ms.toFixed(1)} ms</span>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        )}

        {/* Statistics Tab */}
        {activeTab === 'stats' && (
          <div className="space-y-3">
            <div className="rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] overflow-hidden">
              <table className="w-full text-left text-xs">
                <thead className="border-b border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-[var(--text-muted)] text-[10px] uppercase font-mono tracking-wider">
                  <tr>
                    {['Target', 'Endpoint', 'Probes', 'Success', 'Loss', 'Avg RTT', 'Min / Max'].map(h => (
                      <th key={h} className="py-2 px-3 font-semibold">{h}</th>
                    ))}
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--border-hairline)] font-mono tabular-nums text-xs">
                  {snapshot?.stats.map(s => (
                    <tr key={s.target_id} className="hover:bg-[var(--bg-panel-hover)] transition-colors">
                      <td className="py-2 px-3 font-sans font-medium text-[var(--text-primary)]">{s.target_label}</td>
                      <td className="py-2 px-3 text-[var(--text-muted)]">{s.host}</td>
                      <td className="py-2 px-3 text-[var(--text-primary)]">{s.total_probes}</td>
                      <td className="py-2 px-3 text-emerald-400 font-semibold">{s.success_count}</td>
                      <td className={`py-2 px-3 font-bold ${s.loss_percent > 0 ? 'text-rose-400' : 'text-emerald-400'}`}>
                        {s.loss_percent.toFixed(1)}%
                      </td>
                      <td className="py-2 px-3 text-[var(--text-primary)] font-semibold">{s.avg_rtt_ms.toFixed(2)} ms</td>
                      <td className="py-2 px-3 text-[var(--text-muted)]">{s.min_rtt_ms.toFixed(1)} / {s.max_rtt_ms.toFixed(1)} ms</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>

            {/* Historical Metrics Visualization Panel */}
            <div className="p-4 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2.5">
                <div className="flex items-center space-x-2">
                  <TrendingUp className="h-4 w-4 text-sky-400" />
                  <div>
                    <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Latency &amp; Loss Time-Series</h3>
                    <p className="text-[11px] text-[var(--text-muted)]">Target aggregation over selectable time windows</p>
                  </div>
                </div>

                <div className="flex flex-wrap items-center gap-2">
                  {/* Target selector */}
                  <select
                    value={selectedTargetId}
                    onChange={e => setSelectedTargetId(e.target.value)}
                    className="px-2.5 py-1 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-[var(--text-primary)] text-xs font-mono outline-none focus:border-sky-500"
                  >
                    {snapshot?.stats.map(s => (
                      <option key={s.target_id} value={s.target_id}>
                        {s.target_label} ({s.host})
                      </option>
                    ))}
                  </select>

                  {/* Time Window Buttons */}
                  <div className="flex rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] p-0.5 text-xs font-mono">
                    {[
                      { label: '15m', val: 15 },
                      { label: '1h', val: 60 },
                      { label: '6h', val: 360 },
                      { label: '24h', val: 1440 },
                    ].map(w => (
                      <button
                        key={w.val}
                        onClick={() => setTimeWindowMin(w.val)}
                        className={`px-2 py-0.5 rounded transition-colors ${
                          timeWindowMin === w.val
                            ? 'bg-[var(--border-hairline)] text-[var(--text-primary)] font-semibold'
                            : 'text-[var(--text-muted)] hover:text-[var(--text-primary)]'
                        }`}
                      >
                        {w.label}
                      </button>
                    ))}
                  </div>

                  {/* Manual Refresh */}
                  <button
                    onClick={fetchTimeSeries}
                    className="p-1 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-colors"
                    title="Refresh telemetry chart"
                  >
                    <RefreshCw className={`h-3.5 w-3.5 ${loadingTimeSeries ? 'animate-spin' : ''}`} />
                  </button>
                </div>
              </div>

              {/* Chart Component */}
              <HistoricalChart
                timeSeries={timeSeriesData}
                targetLabel={snapshot?.stats.find(s => s.target_id === selectedTargetId)?.target_label ?? selectedTargetId}
              />
            </div>
          </div>
        )}

        {/* Outages Tab */}
        {activeTab === 'outages' && (
          <div className="rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] p-4">
            {!snapshot?.recent_outages?.length ? (
              <div className="text-center py-8 text-[var(--text-muted)] text-xs font-mono">
                <CheckCircle2 className="h-6 w-6 text-emerald-500 mx-auto mb-2 opacity-80" />
                <p>Zero outage events recorded. All probes continuous and healthy.</p>
              </div>
            ) : (
              <div className="space-y-2">
                {snapshot.recent_outages.map(o => (
                  <div key={o.id} className="p-3 rounded-md bg-rose-500/10 border border-rose-500/20 text-xs flex items-center justify-between">
                    <div>
                      <p className="font-bold text-rose-300">{o.summary}</p>
                      <p className="text-[11px] font-mono text-[var(--text-muted)] mt-0.5">Affected: {o.affected_targets}</p>
                    </div>
                    <span className="font-mono tabular-nums text-rose-400 font-semibold text-xs">
                      {o.duration_seconds ? `${o.duration_seconds}s` : 'Active'}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* Custom Targets Tab */}
        {activeTab === 'targets' && (
          <div className="space-y-3">
            {/* Quick-Add Popular Presets */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-2.5">
              <div className="flex items-center space-x-2">
                <Layers className="h-4 w-4 text-sky-400" />
                <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Quick Telemetry Presets</h3>
              </div>

              <div className="grid grid-cols-2 sm:grid-cols-4 gap-2">
                {[
                  { label: 'Zoom Meetings', host: 'zoom.us', cat: 'Work' },
                  { label: 'Microsoft Teams', host: 'teams.microsoft.com', cat: 'Work' },
                  { label: 'Discord Voice', host: 'gateway.discord.gg', cat: 'Voice' },
                  { label: 'Steam Gaming (Frankfurt)', host: '155.133.226.68', cat: 'Gaming' },
                ].map(p => {
                  const alreadyAdded = snapshot?.custom_targets?.some(ct => ct.host.toLowerCase() === p.host.toLowerCase());
                  return (
                    <button
                      key={p.host}
                      disabled={alreadyAdded || addLoading}
                      onClick={async () => {
                        try {
                          await invoke('add_custom_target', { label: p.label, host: p.host });
                          await fetchSnapshot();
                          showNotice(`Preset active: ${p.label}`);
                        } catch (err) {
                          showNotice(String(err), 'err');
                        }
                      }}
                      className={`p-2.5 rounded-md border text-left transition-colors ${
                        alreadyAdded
                          ? 'opacity-40 border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] cursor-default'
                          : 'border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:border-[var(--border-subtle)] hover:bg-[var(--bg-panel-hover)]'
                      }`}
                    >
                      <div className="flex justify-between items-center">
                        <span className="text-[9px] font-mono font-bold uppercase tracking-wider text-[var(--text-muted)]">{p.cat}</span>
                        {alreadyAdded && <Check className="h-3 w-3 text-emerald-400" />}
                      </div>
                      <p className="text-xs font-medium text-[var(--text-primary)] mt-0.5 truncate">{p.label}</p>
                      <p className="text-[10px] font-mono text-[var(--text-muted)] truncate">{p.host}</p>
                    </button>
                  );
                })}
              </div>
            </div>

            {/* Add Target Form */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)]">
              <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)] mb-2.5 flex items-center space-x-1.5">
                <Plus className="h-3.5 w-3.5 text-sky-400" />
                <span>Add Custom Target</span>
              </h3>
              <div className="flex flex-col sm:flex-row gap-2">
                <input
                  value={addLabel}
                  onChange={e => { setAddLabel(e.target.value); setAddError(null); }}
                  onKeyDown={e => e.key === 'Enter' && handleAddTarget()}
                  placeholder="Target label (e.g. Local Gateway / NAS)"
                  className="flex-1 px-3 py-1.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] outline-none focus:border-sky-500 font-sans"
                />
                <input
                  value={addHost}
                  onChange={e => { setAddHost(e.target.value); setAddError(null); }}
                  onKeyDown={e => e.key === 'Enter' && handleAddTarget()}
                  placeholder="Hostname or IP (e.g. 192.168.1.1)"
                  className="flex-1 px-3 py-1.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] outline-none focus:border-sky-500 font-mono"
                />
                <button
                  onClick={handleAddTarget}
                  disabled={addLoading}
                  className="px-3.5 py-1.5 rounded-md border border-sky-500/30 bg-sky-500/15 hover:bg-sky-500/25 text-sky-200 text-xs font-medium flex items-center justify-center space-x-1.5 disabled:opacity-50 transition-colors shrink-0">
                  <Plus className="h-3.5 w-3.5" />
                  <span>{addLoading ? 'Adding…' : 'Add Target'}</span>
                </button>
              </div>
              {addError && (
                <p className="mt-2 text-xs text-rose-400 flex items-center space-x-1 font-mono">
                  <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                  <span>{addError}</span>
                </p>
              )}
            </div>

            {/* Target Table */}
            <div className="rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] overflow-hidden">
              {!snapshot?.custom_targets?.length ? (
                <div className="p-8 text-center text-xs text-[var(--text-muted)] font-mono">
                  <Globe className="h-6 w-6 mx-auto mb-2 opacity-40 text-sky-400" />
                  <p>No custom targets configured. Add an endpoint above to monitor it.</p>
                </div>
              ) : (
                <table className="w-full text-left text-xs">
                  <thead className="border-b border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-[var(--text-muted)] text-[10px] uppercase font-mono tracking-wider">
                    <tr>
                      <th className="py-2 px-3">State</th>
                      <th className="py-2 px-3">Name</th>
                      <th className="py-2 px-3">Host</th>
                      <th className="py-2 px-3">RTT</th>
                      <th className="py-2 px-3">Loss</th>
                      <th className="py-2 px-3 text-right">Actions</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-[var(--border-hairline)] font-mono tabular-nums text-xs">
                    {snapshot.custom_targets.map(ct => {
                      const latestObs = [...(snapshot.latest_observations ?? [])]
                        .reverse().find(o => o.target_id === ct.target_id);
                      const stat = snapshot.stats.find(s => s.target_id === ct.target_id);
                      const rtt = latestObs?.rtt_measured_us
                        ? (latestObs.rtt_measured_us / 1000).toFixed(1) + ' ms' : '—';

                      return (
                        <tr key={ct.target_id} className={`hover:bg-[var(--bg-panel-hover)] transition-colors ${!ct.enabled ? 'opacity-50' : ''}`}>
                          <td className="py-2 px-3">
                            <span className={`inline-flex items-center px-1.5 py-0.2 rounded text-[10px] font-mono ${
                              ct.enabled
                                ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/30'
                                : 'bg-[var(--bg-panel-hover)] text-[var(--text-muted)] border border-[var(--border-hairline)]'
                            }`}>
                              {ct.enabled ? 'ACTIVE' : 'PAUSED'}
                            </span>
                          </td>
                          <td className="py-2 px-3 font-sans font-medium text-[var(--text-primary)]">{ct.label}</td>
                          <td className="py-2 px-3 text-[var(--text-muted)]">{ct.host}</td>
                          <td className="py-2 px-3 text-[var(--text-primary)] font-semibold">{ct.enabled ? rtt : '—'}</td>
                          <td className={`py-2 px-3 font-bold ${(stat?.loss_percent ?? 0) > 0 ? 'text-rose-400' : 'text-emerald-400'}`}>
                            {stat ? `${stat.loss_percent.toFixed(1)}%` : '—'}
                          </td>
                          <td className="py-2 px-3 text-right">
                            <div className="flex justify-end space-x-1.5">
                              <button
                                onClick={() => handleToggleEnabled(ct)}
                                title={ct.enabled ? 'Pause telemetry' : 'Resume telemetry'}
                                className="p-1 rounded border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] text-[var(--text-muted)] hover:text-[var(--text-primary)] transition-colors">
                                {ct.enabled
                                  ? <ToggleRight className="h-3.5 w-3.5 text-emerald-400" />
                                  : <ToggleLeft className="h-3.5 w-3.5 text-[var(--text-muted)]" />}
                              </button>
                              <button
                                onClick={() => handleRemove(ct)}
                                title="Remove target"
                                className="p-1 rounded border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:border-rose-500/40 hover:bg-rose-500/10 text-[var(--text-muted)] hover:text-rose-400 transition-colors">
                                <Trash2 className="h-3.5 w-3.5" />
                              </button>
                            </div>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              )}
            </div>

            <p className="text-[11px] font-mono text-[var(--text-muted)] px-1">
              Gateway, Cloudflare, and Google DNS are core system targets. User targets are persisted in SQLite.
            </p>
          </div>
        )}
                {/* Tools & Diagnostic Suite Tab */}
        {activeTab === 'tools' && (
          <div className="space-y-3">
            {/* 1-Click Network Stack Repair Tool */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2.5">
                <div className="flex items-center space-x-2">
                  <LifeBuoy className="h-4 w-4 text-sky-400" />
                  <div>
                    <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Network Stack Repair</h3>
                    <p className="text-[11px] text-[var(--text-muted)]">Flush resolver cache, clear ARP tables, and renew DHCP lease</p>
                  </div>
                </div>

                <button
                  disabled={runningRepair}
                  onClick={handleExecuteRepair}
                  className="px-3 py-1.5 rounded-md border border-sky-500/30 bg-sky-500/15 hover:bg-sky-500/25 text-sky-200 text-xs font-medium transition-colors flex items-center space-x-1.5 disabled:opacity-50 shrink-0"
                >
                  <RefreshCw className={`h-3 w-3 ${runningRepair ? 'animate-spin' : ''}`} />
                  <span>{runningRepair ? 'Repairing Stack...' : 'Run Network Repair'}</span>
                </button>
              </div>

              {repairResult ? (
                <div className="space-y-2 pt-1 font-mono text-xs">
                  <div className={`p-2.5 rounded-md border flex items-center space-x-2 ${
                    repairResult.overall_success
                      ? 'bg-emerald-500/10 border-emerald-500/30 text-emerald-300'
                      : 'bg-amber-500/10 border-amber-500/30 text-amber-300'
                  }`}>
                    <Check className="h-3.5 w-3.5 shrink-0" />
                    <span className="font-sans">{repairResult.summary}</span>
                  </div>

                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                    {repairResult.steps.map((st, idx) => (
                      <div key={idx} className="p-2.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] space-y-1">
                        <div className="flex justify-between items-center">
                          <span className="text-xs font-medium text-[var(--text-primary)] font-sans">{st.step_name}</span>
                          <span className={`px-1.5 py-0.2 rounded text-[10px] font-mono font-bold uppercase ${
                            st.success ? 'bg-emerald-500/15 text-emerald-400 border border-emerald-500/30' : 'bg-rose-500/15 text-rose-400 border border-rose-500/30'
                          }`}>
                            {st.success ? 'OK' : 'WARN'}
                          </span>
                        </div>
                        <p className="text-[10px] font-mono text-sky-400">{st.command}</p>
                        <p className="text-[11px] text-[var(--text-muted)] font-mono truncate">{st.output}</p>
                      </div>
                    ))}
                  </div>
                </div>
              ) : (
                <div className="p-3.5 rounded-md border border-dashed border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-center text-[11px] text-[var(--text-muted)] font-mono">
                  Executes <code className="text-[var(--text-primary)]">ipconfig /flushdns</code>, <code className="text-[var(--text-primary)]">/registerdns</code>, and DHCP renewal to clear stale routes.
                </div>
              )}
            </div>

            {/* DNS Speed Benchmark & Resolver Comparison */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2.5">
                <div className="flex items-center space-x-2">
                  <Compass className="h-4 w-4 text-sky-400" />
                  <div>
                    <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">DNS Resolver Benchmark</h3>
                    <p className="text-[11px] text-[var(--text-muted)]">Compare live lookup latency against primary global resolvers</p>
                  </div>
                </div>

                <button
                  disabled={runningDnsBench}
                  onClick={handleRunDnsBenchmark}
                  className="px-3 py-1.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] text-[var(--text-primary)] text-xs font-medium transition-colors flex items-center space-x-1.5 disabled:opacity-50 shrink-0"
                >
                  <RefreshCw className={`h-3 w-3 ${runningDnsBench ? 'animate-spin' : ''}`} />
                  <span>{runningDnsBench ? 'Querying...' : 'Run Benchmark'}</span>
                </button>
              </div>

              {dnsBenchResult ? (
                <div className="space-y-2.5 pt-1">
                  <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-sky-500/30 text-xs flex items-start space-x-2">
                    <Zap className="h-3.5 w-3.5 text-sky-400 mt-0.5 shrink-0" />
                    <div>
                      <p className="font-bold text-[var(--text-primary)]">Fastest Resolver: {dnsBenchResult.fastest_resolver_name}</p>
                      <p className="text-[var(--text-primary)] text-[11px] mt-0.5 leading-relaxed">{dnsBenchResult.recommendation}</p>
                    </div>
                  </div>

                  <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-5 gap-2">
                    {dnsBenchResult.resolvers.map(r => (
                      <div
                        key={r.id}
                        className={`p-2.5 rounded-md border relative ${
                          r.is_fastest
                            ? 'bg-[var(--bg-panel-hover)] border-sky-500/50'
                            : 'bg-[var(--bg-panel-subtle)] border-[var(--border-hairline)]'
                        }`}
                      >
                        {r.is_fastest && (
                          <span className="absolute top-2 right-2 px-1.5 py-0.2 rounded text-[9px] font-mono font-bold uppercase bg-sky-500/20 text-sky-300 border border-sky-500/40">
                            FASTEST
                          </span>
                        )}
                        <p className="text-xs font-medium text-[var(--text-primary)]">{r.name}</p>
                        <p className="text-[10px] font-mono text-[var(--text-muted)]">{r.server_ip}</p>
                        <div className="mt-2 font-mono tabular-nums">
                          <p className="text-xl font-black text-[var(--text-primary)]">
                            {r.avg_latency_ms < 900 ? `${r.avg_latency_ms.toFixed(1)}` : '—'}
                            <span className="text-[11px] font-normal text-[var(--text-muted)] ml-1">ms</span>
                          </p>
                          <p className="text-[10px] text-[var(--text-muted)] mt-0.5">
                            Min: {r.min_latency_ms.toFixed(0)}ms &bull; Max: {r.max_latency_ms.toFixed(0)}ms
                          </p>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              ) : (
                <div className="p-4 rounded-md border border-dashed border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-center text-[11px] text-[var(--text-muted)] font-mono">
                  Benchmark lookup latency across Cloudflare, Google DNS, Quad9, and OpenDNS.
                </div>
              )}
            </div>

            {/* Wi-Fi Signal & Link Speed Inspector */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center space-x-2">
                  <Wifi className="h-4 w-4 text-sky-400" />
                  <div>
                    <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Wi-Fi &amp; Channel Telemetry</h3>
                    <p className="text-[11px] text-[var(--text-muted)]">Live signal strength, frequency band, channel congestion, and link rate</p>
                  </div>
                </div>

                <button
                  disabled={fetchingWifi}
                  onClick={handleFetchWifi}
                  className="p-1 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] text-xs text-[var(--text-muted)] hover:text-[var(--text-primary)] flex items-center space-x-1 transition-colors"
                  title="Refresh Wi-Fi Status"
                >
                  <RefreshCw className={`h-3 w-3 ${fetchingWifi ? 'animate-spin' : ''}`} />
                  <span>Refresh</span>
                </button>
              </div>

              {wifiDetails && wifiDetails.is_connected ? (
                <div className="space-y-3">
                  <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-2 font-mono tabular-nums text-xs">
                    <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                      <p className="text-[10px] uppercase font-mono font-bold text-[var(--text-muted)]">SSID</p>
                      <p className="text-xs font-sans font-bold text-[var(--text-primary)] truncate mt-0.5">{wifiDetails.ssid}</p>
                    </div>

                    <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                      <p className="text-[10px] uppercase font-mono font-bold text-[var(--text-muted)]">Signal</p>
                      <p className={`text-base font-black mt-0.5 ${
                        wifiDetails.signal_percent >= 75 ? 'text-emerald-400' : wifiDetails.signal_percent >= 50 ? 'text-amber-400' : 'text-rose-400'
                      }`}>
                        {wifiDetails.signal_percent}%
                      </p>
                    </div>

                    <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                      <p className="text-[10px] uppercase font-mono font-bold text-[var(--text-muted)]">Band</p>
                      <p className="text-xs font-bold text-sky-400 mt-0.5">{wifiDetails.band}</p>
                    </div>

                    <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                      <p className="text-[10px] uppercase font-mono font-bold text-[var(--text-muted)]">Channel</p>
                      <p className="text-xs font-bold text-[var(--text-primary)] mt-0.5">Ch {wifiDetails.channel || '—'}</p>
                    </div>

                    <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                      <p className="text-[10px] uppercase font-mono font-bold text-[var(--text-muted)]">Link Rate</p>
                      <p className="text-xs font-bold text-emerald-400 mt-0.5">
                        {wifiDetails.link_speed_mbps ? `${wifiDetails.link_speed_mbps} Mbps` : '—'}
                      </p>
                    </div>

                    <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                      <p className="text-[10px] uppercase font-mono font-bold text-[var(--text-muted)]">Security</p>
                      <p className="text-xs font-sans text-[var(--text-primary)] truncate mt-0.5">{wifiDetails.security || 'WPA2/WPA3'}</p>
                    </div>
                  </div>

                  <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)] text-xs text-[var(--text-primary)] flex items-center space-x-2">
                    <ShieldCheck className="h-4 w-4 text-sky-400 shrink-0" />
                    <span className="font-sans">{wifiScan?.interference_verdict || wifiDetails.health_verdict}</span>
                  </div>

                  {/* Channel Congestion Map */}
                  {wifiScan && wifiScan.channel_congestion && wifiScan.channel_congestion.length > 0 && (
                    <div className="pt-1 space-y-2">
                      <span className="text-[11px] font-mono font-bold uppercase tracking-wider text-[var(--text-muted)] block">
                        Channel Congestion Map ({wifiScan.neighbors.length} Networks Detected)
                      </span>
                      <div className="grid grid-cols-4 sm:grid-cols-6 lg:grid-cols-12 gap-1.5 font-mono">
                        {wifiScan.channel_congestion.map((c) => (
                          <div
                            key={`${c.band}-${c.channel}`}
                            className={`p-1.5 rounded-md border text-center transition-colors ${
                              c.is_your_channel
                                ? 'border-sky-500/60 bg-[var(--bg-panel-hover)]'
                                : 'border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)]'
                            }`}
                          >
                            <span className="text-[9px] text-[var(--text-muted)] block">Ch {c.channel}</span>
                            <span
                              className={`text-xs font-black block my-0.5 tabular-nums ${
                                c.network_count > 3
                                  ? 'text-rose-400'
                                  : c.network_count > 1
                                  ? 'text-amber-400'
                                  : 'text-emerald-400'
                              }`}
                            >
                              {c.network_count}
                            </span>
                            <span className="text-[8px] text-[var(--text-muted)] block truncate">{c.band.split(' ')[0]}</span>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}

                  {/* Neighboring Access Points Table */}
                  {wifiScan && wifiScan.neighbors && wifiScan.neighbors.length > 0 && (
                    <div className="pt-1">
                      <p className="text-[11px] font-mono font-bold uppercase tracking-wider text-[var(--text-muted)] mb-1.5">
                        Nearby Wireless Networks
                      </p>
                      <div className="max-h-40 overflow-y-auto rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-xs">
                        <table className="w-full text-left font-mono tabular-nums">
                          <thead className="bg-[var(--bg-panel)] text-[var(--text-muted)] text-[10px] uppercase border-b border-[var(--border-hairline)]">
                            <tr>
                              <th className="py-1.5 px-2.5">SSID</th>
                              <th className="py-1.5 px-2.5">Signal</th>
                              <th className="py-1.5 px-2.5">Channel</th>
                              <th className="py-1.5 px-2.5">Band</th>
                              <th className="py-1.5 px-2.5">Security</th>
                            </tr>
                          </thead>
                          <tbody className="divide-y divide-[var(--border-hairline)] text-xs">
                            {wifiScan.neighbors.map((n, idx) => (
                              <tr
                                key={`${n.bssid}-${idx}`}
                                className={n.is_current ? 'bg-[var(--bg-panel-hover)] font-medium' : 'hover:bg-[var(--bg-panel-hover)]/40'}
                              >
                                <td className="py-1.5 px-2.5 flex items-center space-x-1.5 truncate max-w-[150px] font-sans">
                                  {n.is_current && <Check className="h-3 w-3 text-emerald-400 shrink-0" />}
                                  <span className="truncate text-[var(--text-primary)]">{n.ssid || '<Hidden SSID>'}</span>
                                </td>
                                <td className="py-1.5 px-2.5">
                                  <span
                                    className={
                                      n.signal_percent >= 70
                                        ? 'text-emerald-400'
                                        : n.signal_percent >= 40
                                        ? 'text-amber-400'
                                        : 'text-[var(--text-muted)]'
                                    }
                                  >
                                    {n.signal_percent}%
                                  </span>
                                </td>
                                <td className="py-1.5 px-2.5 text-[var(--text-primary)]">Ch {n.channel}</td>
                                <td className="py-1.5 px-2.5 text-[var(--text-muted)]">{n.band}</td>
                                <td className="py-1.5 px-2.5 text-[var(--text-muted)] truncate max-w-[100px] font-sans">{n.security}</td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </div>
                    </div>
                  )}
                </div>
              ) : (
                <div className="p-3.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)] text-xs text-[var(--text-muted)] font-mono">
                  Ethernet connection is primary, or no active Wi-Fi adapter detected.
                </div>
              )}
            </div>

            {/* Visual Multi-Hop Traceroute (MTR) */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2.5">
                <div className="flex items-center space-x-2">
                  <Network className="h-4 w-4 text-sky-400" />
                  <div>
                    <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Visual Multi-Hop Traceroute</h3>
                    <p className="text-[11px] text-[var(--text-muted)]">Pinpoint transit router drops and latency jumps hop-by-hop</p>
                  </div>
                </div>

                <div className="flex items-center space-x-2">
                  <input
                    value={tracerouteTarget}
                    onChange={e => setTracerouteTarget(e.target.value)}
                    placeholder="Host or IP (e.g. 1.1.1.1)"
                    className="px-2.5 py-1 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-xs font-mono text-[var(--text-primary)] outline-none focus:border-sky-500"
                  />
                  <button
                    disabled={runningTraceroute || !tracerouteTarget.trim()}
                    onClick={handleRunTraceroute}
                    className="px-3 py-1 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] text-[var(--text-primary)] text-xs font-medium transition-colors flex items-center space-x-1.5 disabled:opacity-50"
                  >
                    <Navigation className={`h-3 w-3 ${runningTraceroute ? 'animate-spin' : ''}`} />
                    <span>{runningTraceroute ? 'Tracing...' : 'Trace'}</span>
                  </button>
                </div>
              </div>

              {tracerouteResult && (
                <div className="space-y-2 pt-1 font-mono text-xs">
                  <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)] px-1">
                    <span>TARGET: <strong className="text-[var(--text-primary)]">{tracerouteResult.target}</strong> ({tracerouteResult.destination_ip})</span>
                    <span>HOPS: <strong className="text-sky-400">{tracerouteResult.total_hops}</strong></span>
                  </div>

                  <div className="rounded-md border border-[var(--border-hairline)] overflow-hidden bg-[var(--bg-panel-subtle)]">
                    <table className="w-full text-left font-mono tabular-nums">
                      <thead className="border-b border-[var(--border-hairline)] bg-[var(--bg-panel)] text-[var(--text-muted)] text-[10px] uppercase">
                        <tr>
                          <th className="py-1.5 px-2.5 w-12 text-center">Hop</th>
                          <th className="py-1.5 px-2.5">Router IP</th>
                          <th className="py-1.5 px-2.5 text-right">Round-Trip</th>
                          <th className="py-1.5 px-2.5 text-right">Status</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-[var(--border-hairline)]">
                        {tracerouteResult.hops.map(h => (
                          <tr key={h.hop_number} className="hover:bg-[var(--bg-panel-hover)]/40">
                            <td className="py-1.5 px-2.5 text-center font-bold text-[var(--text-muted)]">{h.hop_number}</td>
                            <td className="py-1.5 px-2.5 font-semibold text-[var(--text-primary)]">
                              {h.ip ? h.ip : <span className="text-[var(--text-muted)]">* * * (Timeout)</span>}
                            </td>
                            <td className="py-1.5 px-2.5 text-right font-bold text-sky-400">
                              {h.rtt_ms != null ? `${h.rtt_ms.toFixed(1)} ms` : '—'}
                            </td>
                            <td className="py-1.5 px-2.5 text-right">
                              <span className={`inline-flex items-center px-1.5 py-0.2 rounded text-[9px] font-bold uppercase ${
                                h.is_destination
                                  ? 'bg-emerald-500/15 text-emerald-400 border border-emerald-500/30'
                                  : h.ip
                                    ? 'bg-[var(--bg-panel-hover)] text-[var(--text-primary)] border border-[var(--border-hairline)]'
                                    : 'bg-rose-500/15 text-rose-400 border border-rose-500/30'
                              }`}>
                                {h.status}
                              </span>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              )}
            </div>

            {/* Bufferbloat Stress Test Card */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2.5">
                <div className="flex items-center space-x-2">
                  <Activity className="h-4 w-4 text-emerald-400" />
                  <div>
                    <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Bufferbloat Latency Benchmark</h3>
                    <p className="text-[11px] text-[var(--text-muted)]">Measures latency degradation during maximum concurrent download saturation</p>
                  </div>
                </div>

                <button
                  disabled={runningBufferbloat}
                  onClick={handleRunBufferbloat}
                  className="px-3 py-1.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] text-[var(--text-primary)] text-xs font-medium transition-colors flex items-center space-x-1.5 disabled:opacity-50 shrink-0"
                >
                  <Zap className={`h-3 w-3 ${runningBufferbloat ? 'animate-spin' : ''}`} />
                  <span>{runningBufferbloat ? 'Saturating...' : 'Run Stress Test'}</span>
                </button>
              </div>

              {bufferbloatResult ? (
                <div className="space-y-2 pt-1 font-mono tabular-nums text-xs">
                  <div className="p-3 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] flex flex-col sm:flex-row sm:items-center justify-between gap-3">
                    <div className="flex items-center space-x-3">
                      <div className="text-xl font-black px-2.5 py-1 rounded border border-[var(--border-hairline)] bg-[var(--bg-panel-hover)] text-[var(--text-primary)]">
                        {bufferbloatResult.grade}
                      </div>
                      <div>
                        <div className="text-xs font-bold font-sans text-[var(--text-primary)]">{bufferbloatResult.grade_summary}</div>
                        <div className="text-[11px] text-[var(--text-muted)]">
                          Saturation throughput: {bufferbloatResult.download_mbps} Mbps
                        </div>
                      </div>
                    </div>

                    <div className="flex items-center space-x-3 text-xs">
                      <div>
                        <span className="block text-[9px] text-[var(--text-muted)] uppercase">Baseline Ping</span>
                        <span className="font-bold text-[var(--text-primary)]">{bufferbloatResult.baseline_ping_ms} ms</span>
                      </div>
                      <div className="text-[var(--text-dim)] font-sans">→</div>
                      <div>
                        <span className="block text-[9px] text-[var(--text-muted)] uppercase">Loaded Ping</span>
                        <span className="font-bold text-amber-400">{bufferbloatResult.loaded_ping_ms} ms</span>
                      </div>
                      <div className="text-[var(--text-dim)] font-sans">=</div>
                      <div>
                        <span className="block text-[9px] text-[var(--text-muted)] uppercase">Bloat Delta</span>
                        <span className="font-bold text-emerald-400">+{bufferbloatResult.bufferbloat_delta_ms} ms</span>
                      </div>
                    </div>
                  </div>
                </div>
              ) : (
                <div className="p-3.5 rounded-md border border-dashed border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-center text-[11px] text-[var(--text-muted)] font-mono">
                  Benchmark queue bloat under heavy network throughput.
                </div>
              )}
            </div>

            {/* Gaming Server Region Ping Radar */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2.5">
                <div className="flex items-center space-x-2">
                  <Gamepad2 className="h-4 w-4 text-sky-400" />
                  <div>
                    <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Game Server Latency Radar</h3>
                    <p className="text-[11px] text-[var(--text-muted)]">Probe regional datacenters for Valorant, CS2, Fortnite, and Discord Voice</p>
                  </div>
                </div>

                <button
                  disabled={scanningGameRadar}
                  onClick={handleRunGameRadar}
                  className="px-3 py-1.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] text-[var(--text-primary)] text-xs font-medium transition-colors flex items-center space-x-1.5 disabled:opacity-50 shrink-0"
                >
                  <RefreshCw className={`h-3 w-3 ${scanningGameRadar ? 'animate-spin' : ''}`} />
                  <span>{scanningGameRadar ? 'Probing...' : 'Scan Regions'}</span>
                </button>
              </div>

              {gameRadarResult ? (
                <div className="space-y-2.5 pt-1">
                  <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-sky-500/30 text-xs text-[var(--text-primary)] flex items-center space-x-2 font-mono">
                    <Zap className="h-3.5 w-3.5 text-sky-400 shrink-0" />
                    <span>
                      Optimal Datacenter: <strong className="text-[var(--text-primary)] font-sans">{gameRadarResult.lowest_ping_region}</strong> at{' '}
                      <strong className="text-emerald-400">{gameRadarResult.lowest_ping_ms.toFixed(0)} ms</strong>.
                    </span>
                  </div>

                  <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-2 font-mono tabular-nums">
                    {gameRadarResult.pings.map((p, idx) => (
                      <div
                        key={idx}
                        className={`p-2.5 rounded-md border relative ${
                          idx === 0
                            ? 'bg-[var(--bg-panel-hover)] border-sky-500/50'
                            : 'bg-[var(--bg-panel-subtle)] border-[var(--border-hairline)]'
                        }`}
                      >
                        <div className="flex justify-between items-start">
                          <span className="text-[9px] font-bold uppercase tracking-wider text-[var(--text-muted)]">{p.game_title}</span>
                          <span className={`px-1.5 py-0.2 rounded text-[9px] font-bold uppercase ${
                            p.ping_ms != null && p.ping_ms < 40
                              ? 'bg-emerald-500/15 text-emerald-400 border border-emerald-500/30'
                              : p.ping_ms != null && p.ping_ms < 90
                                ? 'bg-sky-500/15 text-sky-300 border border-sky-500/30'
                                : 'bg-amber-500/15 text-amber-300 border border-amber-500/30'
                          }`}>
                            {p.quality_tier}
                          </span>
                        </div>
                        <p className="text-xs font-sans font-medium text-[var(--text-primary)] mt-1 truncate">{p.region_name}</p>
                        <p className="text-xl font-black text-[var(--text-primary)] mt-1">
                          {p.ping_ms != null ? `${p.ping_ms.toFixed(0)}` : '—'}
                          <span className="text-[11px] font-normal text-[var(--text-muted)] ml-1">ms</span>
                        </p>
                      </div>
                    ))}
                  </div>
                </div>
              ) : (
                <div className="p-4 rounded-md border border-dashed border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-center text-[11px] text-[var(--text-muted)] font-mono">
                  Scan round-trip latency to global game server regions.
                </div>
              )}
            </div>

            {/* Local LAN Device Scanner */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2.5">
                <div className="flex items-center space-x-2">
                  <Users className="h-4 w-4 text-sky-400" />
                  <div>
                    <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Local LAN Device Discovery</h3>
                    <p className="text-[11px] text-[var(--text-muted)]">Enumerate active IPv4 hosts and hardware vendors on current subnet</p>
                  </div>
                </div>

                <button
                  disabled={scanningLan}
                  onClick={handleScanLan}
                  className="px-3 py-1.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] text-[var(--text-primary)] text-xs font-medium transition-colors flex items-center space-x-1.5 disabled:opacity-50 shrink-0"
                >
                  <RefreshCw className={`h-3 w-3 ${scanningLan ? 'animate-spin' : ''}`} />
                  <span>{scanningLan ? 'Scanning...' : 'Scan Subnet'}</span>
                </button>
              </div>

              {lanScanResult ? (
                <div className="space-y-2 pt-1 font-mono text-xs">
                  <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)] px-1">
                    <span>SUBNET: <strong className="text-[var(--text-primary)]">{lanScanResult.subnet}</strong></span>
                    <span>ACTIVE HOSTS: <strong className="text-emerald-400">{lanScanResult.total_devices_found}</strong></span>
                  </div>

                  <div className="rounded-md border border-[var(--border-hairline)] overflow-hidden bg-[var(--bg-panel-subtle)]">
                    <table className="w-full text-left font-mono tabular-nums">
                      <thead className="border-b border-[var(--border-hairline)] bg-[var(--bg-panel)] text-[var(--text-muted)] text-[10px] uppercase">
                        <tr>
                          <th className="py-1.5 px-2.5">Device Name</th>
                          <th className="py-1.5 px-2.5">Type</th>
                          <th className="py-1.5 px-2.5">IP</th>
                          <th className="py-1.5 px-2.5">MAC Address</th>
                          <th className="py-1.5 px-2.5">Vendor</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-[var(--border-hairline)]">
                        {lanScanResult.devices.map((d, idx) => (
                          <tr key={idx} className="hover:bg-[var(--bg-panel-hover)]/40">
                            <td className="py-1.5 px-2.5 font-sans font-medium text-[var(--text-primary)] flex items-center space-x-1.5 truncate max-w-[200px]">
                              {d.is_current_host && (
                                <span className="px-1.5 py-0.2 rounded text-[9px] font-mono font-bold uppercase bg-emerald-500/15 text-emerald-400 border border-emerald-500/30 shrink-0">
                                  THIS PC
                                </span>
                              )}
                              {d.is_gateway && (
                                <span className="px-1.5 py-0.2 rounded text-[9px] font-mono font-bold uppercase bg-sky-500/15 text-sky-400 border border-sky-500/30 shrink-0">
                                  GATEWAY
                                </span>
                              )}
                              <span className="truncate">{d.hostname || d.device_type}</span>
                            </td>
                            <td className="py-1.5 px-2.5 text-[var(--text-muted)] font-sans">{d.device_type}</td>
                            <td className="py-1.5 px-2.5 font-bold text-sky-400">{d.ip}</td>
                            <td className="py-1.5 px-2.5 text-[var(--text-muted)]">{d.mac}</td>
                            <td className="py-1.5 px-2.5 text-[var(--text-primary)] font-sans truncate max-w-[150px]">{d.vendor}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              ) : (
                <div className="p-4 rounded-md border border-dashed border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-center text-[11px] text-[var(--text-muted)] font-mono">
                  Scan local subnet for active connected devices.
                </div>
              )}
            </div>
          </div>
        )}

        {/* Settings Tab */}
        {activeTab === 'settings' && (
          <div className="space-y-3">
            {/* Database Status Panel */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center space-x-2">
                  <Database className="h-4 w-4 text-sky-400" />
                  <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Database Telemetry &amp; Storage</h3>
                </div>
                <button
                  onClick={fetchDbStats}
                  className="p-1 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] text-xs text-[var(--text-muted)] hover:text-[var(--text-primary)] flex items-center space-x-1 transition-colors"
                  title="Refresh stats"
                >
                  <RefreshCw className="h-3 w-3" />
                  <span>Refresh</span>
                </button>
              </div>

              <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-2 font-mono tabular-nums text-xs">
                <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                  <p className="text-[10px] uppercase font-bold text-[var(--text-muted)] mb-0.5">Probe Records</p>
                  <p className="text-base font-bold text-[var(--text-primary)]">
                    {dbStats ? dbStats.probe_count.toLocaleString() : '—'}
                  </p>
                </div>

                <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                  <p className="text-[10px] uppercase font-bold text-[var(--text-muted)] mb-0.5">Outages</p>
                  <p className="text-base font-bold text-amber-400">
                    {dbStats ? dbStats.outage_count.toLocaleString() : '—'}
                  </p>
                </div>

                <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                  <p className="text-[10px] uppercase font-bold text-[var(--text-muted)] mb-0.5">Targets</p>
                  <p className="text-base font-bold text-sky-400">
                    {dbStats ? dbStats.custom_target_count : '—'}
                  </p>
                </div>

                <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                  <p className="text-[10px] uppercase font-bold text-[var(--text-muted)] mb-0.5">DB File Size</p>
                  <p className="text-base font-bold text-[var(--text-primary)]">
                    {dbStats ? (dbStats.db_size_bytes / (1024 * 1024)).toFixed(2) + ' MB' : '—'}
                  </p>
                </div>

                <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                  <p className="text-[10px] uppercase font-bold text-[var(--text-muted)] mb-0.5">WAL Cache</p>
                  <p className="text-base font-bold text-[var(--text-muted)]">
                    {dbStats ? (dbStats.wal_size_bytes / (1024 * 1024)).toFixed(2) + ' MB' : '—'}
                  </p>
                </div>

                <div className="p-2.5 rounded-md bg-[var(--bg-panel-subtle)] border border-[var(--border-hairline)]">
                  <p className="text-[10px] uppercase font-bold text-[var(--text-muted)] mb-0.5">Last Pruning</p>
                  <p className="text-xs font-medium truncate mt-0.5 text-[var(--text-primary)]">
                    {dbStats?.last_cleanup_ms
                      ? new Date(dbStats.last_cleanup_ms).toLocaleTimeString()
                      : 'Never'}
                  </p>
                </div>
              </div>

              {dbStats?.oldest_probe_ms && (
                <p className="text-[11px] font-mono text-[var(--text-muted)]">
                  Earliest retained telemetry: {new Date(dbStats.oldest_probe_ms).toLocaleString()}
                </p>
              )}
            </div>

            {/* Retention Policy Configuration */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex items-center space-x-2">
                <HardDrive className="h-4 w-4 text-sky-400" />
                <div>
                  <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Data Retention &amp; Checkpoint Policy</h3>
                  <p className="text-[11px] text-[var(--text-muted)]">Automated background SQLite WAL checkpointing and row cleanup</p>
                </div>
              </div>

              <div className="space-y-3 text-xs">
                {/* Enable toggle */}
                <div className="flex items-center justify-between py-1.5 border-b border-[var(--border-hairline)]">
                  <div>
                    <p className="font-medium text-[var(--text-primary)]">Automatic Pruning</p>
                    <p className="text-[11px] text-[var(--text-muted)]">Remove metrics exceeding duration or row limits in background</p>
                  </div>
                  <button
                    onClick={() => setRetentionSettings(s => ({ ...s, pruning_enabled: !s.pruning_enabled }))}
                    className="p-1 rounded border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] transition-colors"
                  >
                    {retentionSettings.pruning_enabled ? (
                      <ToggleRight className="h-5 w-5 text-emerald-400" />
                    ) : (
                      <ToggleLeft className="h-5 w-5 text-[var(--text-muted)]" />
                    )}
                  </button>
                </div>

                {/* Max Age */}
                <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 py-1.5 border-b border-[var(--border-hairline)]">
                  <div>
                    <p className="font-medium text-[var(--text-primary)]">Maximum Telemetry History</p>
                    <p className="text-[11px] text-[var(--text-muted)]">Discard raw measurement records older than this threshold</p>
                  </div>
                  <select
                    disabled={!retentionSettings.pruning_enabled}
                    value={retentionSettings.max_age_days}
                    onChange={e => setRetentionSettings(s => ({ ...s, max_age_days: Number(e.target.value) }))}
                    className="px-2.5 py-1 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-[var(--text-primary)] text-xs font-mono outline-none disabled:opacity-40"
                  >
                    <option value={1}>1 Day</option>
                    <option value={3}>3 Days</option>
                    <option value={7}>7 Days (Default)</option>
                    <option value={14}>14 Days</option>
                    <option value={30}>30 Days</option>
                    <option value={0}>No Limit</option>
                  </select>
                </div>

                {/* Max Records */}
                <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 py-1.5 border-b border-[var(--border-hairline)]">
                  <div>
                    <p className="font-medium text-[var(--text-primary)]">Maximum Database Rows</p>
                    <p className="text-[11px] text-[var(--text-muted)]">Upper bound on total SQLite rows; oldest records pruned first</p>
                  </div>
                  <select
                    disabled={!retentionSettings.pruning_enabled}
                    value={retentionSettings.max_records}
                    onChange={e => setRetentionSettings(s => ({ ...s, max_records: Number(e.target.value) }))}
                    className="px-2.5 py-1 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-[var(--text-primary)] text-xs font-mono outline-none disabled:opacity-40"
                  >
                    <option value={10000}>10,000 rows</option>
                    <option value={25000}>25,000 rows</option>
                    <option value={50000}>50,000 rows (Default)</option>
                    <option value={100000}>100,000 rows</option>
                    <option value={0}>No Limit</option>
                  </select>
                </div>

                {/* Actions */}
                <div className="flex flex-wrap items-center justify-between gap-2.5 pt-1">
                  <div className="flex items-center space-x-2">
                    <button
                      onClick={handleSaveRetention}
                      disabled={savingSettings}
                      className="px-3 py-1.5 rounded-md border border-sky-500/30 bg-sky-500/15 hover:bg-sky-500/25 text-sky-200 text-xs font-medium transition-colors disabled:opacity-50"
                    >
                      {savingSettings ? 'Saving...' : 'Save Retention'}
                    </button>
                    <button
                      onClick={handleRunCleanup}
                      disabled={cleaningUp || !retentionSettings.pruning_enabled}
                      className="px-3 py-1.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] text-[var(--text-primary)] text-xs font-medium transition-colors flex items-center space-x-1.5 disabled:opacity-40"
                    >
                      <RefreshCw className={`h-3 w-3 ${cleaningUp ? 'animate-spin' : ''}`} />
                      <span>{cleaningUp ? 'Pruning...' : 'Run Pruning Now'}</span>
                    </button>
                  </div>

                  <p className="text-[11px] font-mono text-[var(--text-muted)]">
                    Pruning runs in 1,000-row incremental batches.
                  </p>
                </div>
              </div>
            </div>

            {/* Probe Interval */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex items-center space-x-2">
                <Activity className="h-4 w-4 text-sky-400" />
                <div>
                  <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Probe Cycle Interval</h3>
                  <p className="text-[11px] text-[var(--text-muted)]">How often to ping each target. Takes effect on next cycle without restart.</p>
                </div>
              </div>
              <div className="space-y-2 text-xs">
                <div className="flex items-center justify-between">
                  <span className="text-[var(--text-muted)]">Interval</span>
                  <span className="font-mono font-bold tabular-nums text-[var(--text-primary)]">
                    {probeIntervalMs >= 1000 ? `${(probeIntervalMs / 1000).toFixed(1)} s` : `${probeIntervalMs} ms`}
                  </span>
                </div>
                <input
                  type="range"
                  min={500}
                  max={10000}
                  step={500}
                  value={probeIntervalMs}
                  onChange={e => setProbeIntervalMs(Number(e.target.value))}
                  className="w-full h-1 rounded-full bg-[var(--border-hairline)] accent-sky-400 cursor-pointer"
                />
                <div className="flex justify-between text-[10px] font-mono text-[var(--text-muted)]">
                  {[500, 1000, 1500, 2000, 5000, 10000].map(v => (
                    <button
                      key={v}
                      onClick={() => setProbeIntervalMs(v)}
                      className={`px-1.5 py-0.5 rounded border transition-colors ${probeIntervalMs === v
                        ? 'border-sky-500/40 bg-sky-500/15 text-sky-300'
                        : 'border-[var(--border-hairline)] hover:border-[var(--border-subtle)] text-[var(--text-muted)] hover:text-[var(--text-primary)]'}`}
                    >
                      {v >= 1000 ? `${v/1000}s` : `${v}ms`}
                    </button>
                  ))}
                </div>
                <div className="pt-1 flex items-center justify-between">
                  <p className="text-[11px] font-mono text-[var(--text-muted)]">
                    {probeIntervalMs <= 500 ? '⚡ Gaming - max resolution' :
                     probeIntervalMs <= 1500 ? '✓ Default - balanced precision & load' :
                     probeIntervalMs <= 5000 ? '🔋 Background logging - reduced CPU' :
                     '🌙 Passive - minimal resource use'}
                  </p>
                  <button
                    onClick={handleSaveProbeInterval}
                    disabled={savingInterval}
                    className="px-3 py-1.5 rounded-md border border-sky-500/30 bg-sky-500/15 hover:bg-sky-500/25 text-sky-200 text-xs font-medium transition-colors disabled:opacity-50 shrink-0"
                  >
                    {savingInterval ? 'Saving…' : 'Apply'}
                  </button>
                </div>
              </div>
            </div>

            {/* Windows Startup & Persistence */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex items-center space-x-2">
                <Radio className="h-4 w-4 text-sky-400" />
                <div>
                  <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Windows Startup &amp; Background Mode</h3>
                  <p className="text-[11px] text-[var(--text-muted)]">Configure background persistence and system tray behavior</p>
                </div>
              </div>

              <div className="space-y-3 text-xs">
                {/* Autostart toggle */}
                <div className="flex items-center justify-between py-1.5 border-b border-[var(--border-hairline)]">
                  <div>
                    <p className="font-medium text-[var(--text-primary)]">Launch on Windows Login</p>
                    <p className="text-[11px] text-[var(--text-muted)]">Register in Windows Run key (HKCU) for automated continuous monitoring</p>
                  </div>
                  <button
                    disabled={savingAutostart}
                    onClick={() => handleToggleAutostart('autostart_enabled')}
                    className="p-1 rounded border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] transition-colors"
                  >
                    {autostartConfig.autostart_enabled ? (
                      <ToggleRight className="h-5 w-5 text-emerald-400" />
                    ) : (
                      <ToggleLeft className="h-5 w-5 text-[var(--text-muted)]" />
                    )}
                  </button>
                </div>

                {/* Start Minimized toggle */}
                <div className="flex items-center justify-between py-1.5">
                  <div>
                    <p className="font-medium text-[var(--text-primary)]">Start Minimized to System Tray</p>
                    <p className="text-[11px] text-[var(--text-muted)]">Probing starts silently in the background without popping main window</p>
                  </div>
                  <button
                    disabled={savingAutostart}
                    onClick={() => handleToggleAutostart('start_minimized')}
                    className="p-1 rounded border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] transition-colors"
                  >
                    {autostartConfig.start_minimized ? (
                      <ToggleRight className="h-5 w-5 text-emerald-400" />
                    ) : (
                      <ToggleLeft className="h-5 w-5 text-[var(--text-muted)]" />
                    )}
                  </button>
                </div>
              </div>
            </div>

            {/* Global Hotkeys Card */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex items-center space-x-2">
                <Keyboard className="h-4 w-4 text-sky-400" />
                <div>
                  <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Global Keyboard Shortcuts</h3>
                  <p className="text-[11px] text-[var(--text-muted)]">System-wide hotkeys operative even when application is minimized to tray</p>
                </div>
              </div>

              <div className="space-y-2">
                {hotkeyConfig.bindings.map(binding => {
                  const meta = HOTKEY_ACTION_LABELS[binding.action];
                  const isRecording = recordingAction === binding.action;
                  return (
                    <div
                      key={binding.action}
                      className="flex items-center justify-between gap-2.5 py-2 px-2.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)]"
                    >
                      <div className="min-w-0 flex-1">
                        <p className="text-xs font-medium text-[var(--text-primary)] truncate">{meta.label}</p>
                        <p className="text-[11px] text-[var(--text-muted)] truncate">{meta.desc}</p>
                      </div>

                      {/* Key badge / recorder */}
                      <button
                        onClick={() => startRecording(binding.action)}
                        title="Click then press key combination"
                        className={`px-2.5 py-1 rounded border text-xs font-mono transition-colors min-w-[110px] text-center tabular-nums ${
                          isRecording
                            ? 'border-sky-500 bg-sky-500/15 text-sky-300 animate-pulse'
                            : binding.accelerator
                              ? 'border-[var(--border-hairline)] bg-[var(--bg-panel-hover)] text-[var(--text-primary)] hover:border-[var(--border-subtle)]'
                              : 'border-dashed border-[var(--border-hairline)] text-[var(--text-muted)] hover:border-sky-500/40'
                        }`}
                      >
                        {isRecording ? 'Record keys…' : (binding.accelerator ?? '+ Add')}
                      </button>

                      {/* Enable toggle */}
                      <button
                        onClick={() => toggleHotkeyEnabled(binding.action)}
                        disabled={!binding.accelerator}
                        title={binding.enabled ? 'Disable shortcut' : 'Enable shortcut'}
                        className="p-1 rounded border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] transition-colors disabled:opacity-30"
                      >
                        {binding.enabled && binding.accelerator ? (
                          <ToggleRight className="h-4 w-4 text-emerald-400" />
                        ) : (
                          <ToggleLeft className="h-4 w-4 text-[var(--text-muted)]" />
                        )}
                      </button>

                      {/* Clear button */}
                      <button
                        onClick={() => clearHotkey(binding.action)}
                        disabled={!binding.accelerator}
                        title="Clear shortcut"
                        className="p-1 rounded border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:border-rose-500/30 hover:bg-rose-500/10 text-[var(--text-muted)] hover:text-rose-400 transition-colors disabled:opacity-30"
                      >
                        <X className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  );
                })}
              </div>

              <div className="flex items-center justify-between gap-2.5 pt-1">
                <p className="text-[11px] font-mono text-[var(--text-muted)]">
                  Select shortcut badge, press keys, then Esc to cancel.
                </p>
                <button
                  onClick={() => handleSaveHotkeys(hotkeyConfig)}
                  disabled={savingHotkeys}
                  className="flex items-center space-x-1.5 px-3 py-1.5 rounded-md border border-sky-500/30 bg-sky-500/15 hover:bg-sky-500/25 text-sky-200 text-xs font-medium transition-colors disabled:opacity-50"
                >
                  <Check className="h-3.5 w-3.5" />
                  <span>{savingHotkeys ? 'Applying…' : 'Apply Hotkeys'}</span>
                </button>
              </div>
            </div>

            {/* In-Game / Desktop Text Overlay Settings Card */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center space-x-2">
                  <Layers className="h-4 w-4 text-sky-400" />
                  <div>
                    <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">In-Game / Desktop HUD Overlay</h3>
                    <p className="text-[11px] text-[var(--text-muted)]">Frameless, click-through floating telemetry HUD with adjustable corner and sizing</p>
                  </div>
                </div>
                <button
                  onClick={handleToggleOverlay}
                  className={`px-2.5 py-1 rounded-md border text-xs font-medium transition-colors flex items-center space-x-1.5 ${
                    overlayOpen
                      ? 'border-rose-500/40 bg-rose-500/15 text-rose-300 hover:bg-rose-500/25'
                      : 'border-sky-500/40 bg-sky-500/15 text-sky-300 hover:bg-sky-500/25'
                  }`}
                >
                  <span>{overlayOpen ? 'Close Overlay' : 'Open Overlay'}</span>
                </button>
              </div>

              <div className="grid grid-cols-1 md:grid-cols-2 gap-3 text-xs">
                {/* Corner Position */}
                <div>
                  <label className="block text-[11px] font-mono text-[var(--text-muted)] mb-1">SCREEN CORNER</label>
                  <div className="grid grid-cols-2 gap-1.5">
                    {[
                      { id: 'top_left', label: 'Top-Left' },
                      { id: 'top_right', label: 'Top-Right' },
                      { id: 'bottom_left', label: 'Bottom-Left' },
                      { id: 'bottom_right', label: 'Bottom-Right' },
                    ].map(corner => (
                      <button
                        key={corner.id}
                        type="button"
                        onClick={() => setOverlayConfig(c => ({ ...c, corner: corner.id as OverlayCorner }))}
                        className={`px-2 py-1.5 text-xs font-mono rounded border transition-colors ${
                          overlayConfig.corner === corner.id
                            ? 'border-sky-500/60 bg-[var(--bg-panel-hover)] text-[var(--text-primary)] font-semibold'
                            : 'border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:border-[var(--border-subtle)]'
                        }`}
                      >
                        {corner.label}
                      </button>
                    ))}
                  </div>
                </div>

                {/* Sizing Controls */}
                <div className="space-y-2">
                  <div>
                    <div className="flex justify-between text-[11px] font-mono text-[var(--text-muted)] mb-0.5">
                      <span>FONT SIZE</span>
                      <span className="text-[var(--text-primary)] font-bold">{overlayConfig.font_size} px</span>
                    </div>
                    <input
                      type="range"
                      min="11"
                      max="32"
                      value={overlayConfig.font_size}
                      onChange={e => setOverlayConfig(c => ({ ...c, font_size: Number(e.target.value) }))}
                      className="w-full accent-sky-400 cursor-pointer"
                    />
                  </div>

                  <div>
                    <div className="flex justify-between text-[11px] font-mono text-[var(--text-muted)] mb-0.5">
                      <span>MARGIN OFFSET</span>
                      <span className="text-[var(--text-primary)] font-bold">{overlayConfig.margin} px</span>
                    </div>
                    <input
                      type="range"
                      min="4"
                      max="80"
                      value={overlayConfig.margin}
                      onChange={e => setOverlayConfig(c => ({ ...c, margin: Number(e.target.value) }))}
                      className="w-full accent-sky-400 cursor-pointer"
                    />
                  </div>
                </div>

                {/* Accent Color */}
                <div>
                  <label className="block text-[11px] font-mono text-[var(--text-muted)] mb-1">ACCENT COLOR</label>
                  <div className="flex items-center space-x-1.5">
                    {['#38bdf8', '#10b981', '#3b82f6', '#f59e0b', '#ef4444', '#ffffff'].map(c => (
                      <button
                        key={c}
                        type="button"
                        onClick={() => setOverlayConfig(prev => ({ ...prev, color: c }))}
                        className={`w-6 h-6 rounded border transition-transform ${
                          overlayConfig.color === c ? 'scale-110 border-white ring-1 ring-sky-400' : 'border-[var(--border-hairline)] hover:scale-105'
                        }`}
                        style={{ backgroundColor: c }}
                        title={c}
                      />
                    ))}
                    <input
                      type="color"
                      value={overlayConfig.color}
                      onChange={e => setOverlayConfig(prev => ({ ...prev, color: e.target.value }))}
                      className="w-7 h-7 rounded border border-[var(--border-hairline)] bg-transparent cursor-pointer p-0"
                      title="Custom Color"
                    />
                    <span className="text-xs font-mono text-[var(--text-muted)] ml-1">{overlayConfig.color}</span>
                  </div>
                </div>

                {/* Opacity */}
                <div>
                  <div className="flex justify-between text-[11px] font-mono text-[var(--text-muted)] mb-0.5">
                    <span>OPACITY</span>
                    <span className="text-[var(--text-primary)] font-bold">{Math.round((overlayConfig.opacity ?? 0.95) * 100)}%</span>
                  </div>
                  <input
                    type="range"
                    min="0.3"
                    max="1"
                    step="0.05"
                    value={overlayConfig.opacity ?? 0.95}
                    onChange={e => setOverlayConfig(c => ({ ...c, opacity: Number(e.target.value) }))}
                    className="w-full accent-sky-400 cursor-pointer"
                  />
                </div>
              </div>

              <div className="flex justify-end pt-2 border-t border-[var(--border-hairline)]">
                <button
                  type="button"
                  onClick={() => handleSaveOverlayConfig(overlayConfig)}
                  disabled={savingOverlay}
                  className="flex items-center space-x-1.5 px-3 py-1.5 rounded-md border border-sky-500/30 bg-sky-500/15 hover:bg-sky-500/25 text-sky-200 text-xs font-medium transition-colors disabled:opacity-50"
                >
                  <Check className="h-3.5 w-3.5" />
                  <span>{savingOverlay ? 'Applying…' : 'Save Overlay Settings'}</span>
                </button>
              </div>
            </div>

            {/* Audio Chimes Configuration Card */}
            <div className="p-3.5 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel)] space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center space-x-2">
                  <Volume2 className="h-4 w-4 text-amber-400" />
                  <div>
                    <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--text-primary)]">Packet Drop Audio Cues</h3>
                    <p className="text-[11px] text-[var(--text-muted)]">Synthesized sound notifications triggered on packet loss spikes</p>
                  </div>
                </div>

                <div className="flex items-center space-x-1.5">
                  <button
                    onClick={() => playPacketLossChime(chimeProfile, 0.4)}
                    className="px-2.5 py-1 rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:bg-[var(--bg-panel-hover)] text-xs text-[var(--text-primary)] transition-colors"
                  >
                    Test Tone
                  </button>

                  <button
                    onClick={handleToggleChime}
                    className={`px-2.5 py-1 rounded-md border text-xs font-medium transition-colors ${
                      chimeEnabled
                        ? 'border-amber-500/40 bg-amber-500/15 text-amber-300'
                        : 'border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] text-[var(--text-muted)]'
                    }`}
                  >
                    {chimeEnabled ? 'Chimes Active' : 'Chimes Muted'}
                  </button>
                </div>
              </div>

              <div>
                <label className="block text-[11px] font-mono text-[var(--text-muted)] mb-1.5">AUDIO PROFILE</label>
                <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-2">
                  {[
                    { id: 'descending_triad', title: 'Descending Triad', desc: 'Soft minor triad (E5 → C5 → A4)' },
                    { id: 'sonar_ping', title: 'Sonar Radar Ping', desc: 'High-frequency radar tone' },
                    { id: 'marimba_alert', title: 'Warm Marimba', desc: 'Resonant percussion chime' },
                    { id: 'urgent_glitch', title: 'Tactical Staccato', desc: 'High-urgency triple pip' },
                  ].map(item => (
                    <button
                      key={item.id}
                      type="button"
                      onClick={() => handleSetChimeProfile(item.id as ChimeProfile)}
                      className={`p-2.5 rounded-md border text-left transition-colors ${
                        chimeProfile === item.id
                          ? 'border-amber-500/60 bg-amber-500/10'
                          : 'border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] hover:border-[var(--border-subtle)]'
                      }`}
                    >
                      <div className="flex items-center justify-between mb-0.5">
                        <span className={`text-xs font-medium ${chimeProfile === item.id ? 'text-amber-300 font-bold' : 'text-[var(--text-primary)]'}`}>
                          {item.title}
                        </span>
                        {chimeProfile === item.id && <Check className="h-3 w-3 text-amber-400" />}
                      </div>
                      <p className="text-[10px] text-[var(--text-muted)]">{item.desc}</p>
                    </button>
                  ))}
                </div>
              </div>
            </div>
          </div>
        )}
      </main>

      {/* Precision Windows Status Bar */}
      <footer className="h-8 border-t border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] px-4 text-[11px] font-mono tabular-nums flex items-center justify-between text-[var(--text-muted)] select-none">
        <div className="flex items-center space-x-3">
          <span className="flex items-center space-x-1.5">
            <span className={`h-2 w-2 rounded-full ${snapshot?.is_monitoring ? 'bg-emerald-500' : 'bg-[var(--text-muted)]'}`} />
            <span className="text-[var(--text-primary)]">{snapshot?.is_monitoring ? 'PROBING ACTIVE' : 'PROBING PAUSED'}</span>
          </span>
          <span className="text-[var(--border-subtle)]">•</span>
          <span>ENGINE: WIN32 IP HELPER</span>
          <span className="text-[var(--border-subtle)]">•</span>
          <span>BACKEND: SQLITE MMAP</span>
          <span className="text-[var(--border-subtle)]">•</span>
          <span>TARGETS: {snapshot?.stats?.length ?? 0}</span>
        </div>
        <span>LinkWatch v0.1.0</span>
      </footer>
    </div>
  );
}

// ─── Helpers ─────────────────────────────────────────────────────────────────


// ─── Live Sparkline Component ────────────────────────────────────────────────

interface SparklineProps {
  observations: ProbeObservation[];
  width?: number;
  height?: number;
}


// ─── Historical Chart Component ─────────────────────────────────────────────

interface HistoricalChartProps {
  timeSeries: TargetTimeSeriesResult | null;
  targetLabel: string;
}

function HistoricalChart({ timeSeries, targetLabel }: HistoricalChartProps) {
  const [hoveredPoint, setHoveredPoint] = useState<{
    x: number;
    y: number;
    timeMs: number;
    rttMs: number | null;
    success: boolean;
    ip: string | null;
    lossPercent?: number;
    totalProbes?: number;
  } | null>(null);

  if (!timeSeries || timeSeries.buckets.length === 0) {
    return (
      <div className="h-48 flex flex-col items-center justify-center text-xs text-[var(--text-muted)] font-mono">
        <BarChart2 className="h-6 w-6 mb-2 opacity-30 text-sky-400" />
        <span>No historical telemetry records for this time window.</span>
      </div>
    );
  }

  const svgWidth = 800;
  const svgHeight = 200;
  const padLeft = 45;
  const padRight = 15;
  const padTop = 15;
  const padBottom = 25;

  const plotW = svgWidth - padLeft - padRight;
  const plotH = svgHeight - padTop - padBottom;

  const { start_ms, end_ms, buckets, outages } = timeSeries;
  const timeSpan = Math.max(1, end_ms - start_ms);

  // Determine Y-scale max RTT
  const maxRtt = Math.max(
    20,
    ...buckets.filter(b => b.max_rtt_ms != null).map(b => b.max_rtt_ms!)
  );

  // Convert bucket to coordinates
  const bucketPoints = buckets.map(b => {
    const x = padLeft + ((b.bucket_start_ms - start_ms) / timeSpan) * plotW;
    const y = b.avg_rtt_ms != null
      ? padTop + plotH - (b.avg_rtt_ms / maxRtt) * plotH
      : null;
    return {
      x,
      y,
      bucket: b,
    };
  });

  // Polyline path segments
  const lineSegments: string[] = [];
  let currentSegment = '';
  bucketPoints.forEach(p => {
    if (p.y != null) {
      if (!currentSegment) {
        currentSegment = `M ${p.x.toFixed(1)} ${p.y.toFixed(1)}`;
      } else {
        currentSegment += ` L ${p.x.toFixed(1)} ${p.y.toFixed(1)}`;
      }
    } else {
      if (currentSegment) {
        lineSegments.push(currentSegment);
        currentSegment = '';
      }
    }
  });
  if (currentSegment) {
    lineSegments.push(currentSegment);
  }

  // Y-axis grid levels
  const yTicks = [0, maxRtt * 0.25, maxRtt * 0.5, maxRtt * 0.75, maxRtt];

  return (
    <div className="relative">
      <div className="flex items-center justify-between text-xs font-mono text-[var(--text-muted)] mb-1.5 px-0.5">
        <span className="font-sans font-medium text-[var(--text-primary)]">{targetLabel} - Latency &amp; Loss Telemetry</span>
        <div className="flex items-center space-x-3 text-[11px] tabular-nums">
          <span className="flex items-center space-x-1">
            <span className="h-1.5 w-1.5 rounded-full bg-sky-400 inline-block" />
            <span>RTT</span>
          </span>
          <span className="flex items-center space-x-1">
            <span className="h-1.5 w-1.5 rounded-full bg-rose-500 inline-block" />
            <span>Loss</span>
          </span>
          <span className="flex items-center space-x-1">
            <span className="h-1.5 w-2 rounded-sm bg-rose-500/20 border border-rose-500/40 inline-block" />
            <span>Outage</span>
          </span>
        </div>
      </div>

      <div className="relative overflow-hidden rounded-md border border-[var(--border-hairline)] bg-[var(--bg-panel-subtle)] p-2">
        <svg
          viewBox={`0 0 ${svgWidth} ${svgHeight}`}
          className="w-full h-auto overflow-visible select-none"
          onMouseLeave={() => setHoveredPoint(null)}
        >
          {/* Outage Bands */}
          {outages.map((o, idx) => {
            const oStart = Math.max(start_ms, o.start_ms);
            const oEnd = Math.min(end_ms, o.end_ms ?? end_ms);
            const x1 = padLeft + ((oStart - start_ms) / timeSpan) * plotW;
            const x2 = padLeft + ((oEnd - start_ms) / timeSpan) * plotW;
            const w = Math.max(2, x2 - x1);
            return (
              <rect
                key={`outage-${idx}`}
                x={x1}
                y={padTop}
                width={w}
                height={plotH}
                fill="#EF4444"
                fillOpacity="0.12"
                stroke="#EF4444"
                strokeOpacity="0.3"
                strokeWidth="1"
              />
            );
          })}

          {/* Grid lines and Y labels */}
          {yTicks.map((val, idx) => {
            const y = padTop + plotH - (val / maxRtt) * plotH;
            return (
              <g key={`y-${idx}`}>
                <line
                  x1={padLeft}
                  y1={y}
                  x2={svgWidth - padRight}
                  y2={y}
                  style={{ stroke: 'var(--border-hairline)' }}
                  strokeDasharray="2 2"
                />
                <text
                  x={padLeft - 6}
                  y={y + 3}
                  textAnchor="end"
                  fontSize="9"
                  style={{ fill: 'var(--text-muted)' }}
                  fontFamily="monospace"
                >
                  {val.toFixed(0)}ms
                </text>
              </g>
            );
          })}

          {/* X-axis baseline */}
          <line
            x1={padLeft}
            y1={padTop + plotH}
            x2={svgWidth - padRight}
            y2={padTop + plotH}
            style={{ stroke: 'var(--border-subtle)' }}
            strokeWidth="1"
          />

          {/* Precision 1.5px Stroke Line Segments */}
          {lineSegments.map((d, i) => (
            <path
              key={`seg-${i}`}
              d={d}
              fill="none"
              stroke="#38BDF8"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          ))}

          {/* Loss bars & hover hit targets */}
          {bucketPoints.map((p, idx) => {
            const hasLoss = p.bucket.loss_count > 0;
            return (
              <g key={`b-${idx}`}>
                {/* Loss indicator bar */}
                {hasLoss && (
                  <rect
                    x={p.x - 1.5}
                    y={padTop + plotH - (p.bucket.loss_percent / 100) * plotH}
                    width="3"
                    height={(p.bucket.loss_percent / 100) * plotH}
                    fill="#EF4444"
                    fillOpacity="0.8"
                    rx="0.5"
                  />
                )}

                {/* Point dot on line */}
                {p.y != null && (
                  <circle
                    cx={p.x}
                    cy={p.y}
                    r={hasLoss ? '2.5' : '1.5'}
                    fill={hasLoss ? '#EF4444' : '#10B981'}
                    style={{ stroke: 'var(--bg-panel-subtle)' }}
                    strokeWidth="1"
                  />
                )}

                {/* Invisible hover slice for interaction */}
                <rect
                  x={p.x - Math.max(3, (plotW / buckets.length) / 2)}
                  y={padTop}
                  width={Math.max(6, plotW / buckets.length)}
                  height={plotH}
                  fill="transparent"
                  className="cursor-crosshair"
                  onMouseEnter={() => {
                    setHoveredPoint({
                      x: p.x,
                      y: p.y ?? (padTop + plotH / 2),
                      timeMs: p.bucket.bucket_start_ms,
                      rttMs: p.bucket.avg_rtt_ms,
                      success: p.bucket.success_count > 0,
                      ip: p.bucket.last_resolved_ip,
                      lossPercent: p.bucket.loss_percent,
                      totalProbes: p.bucket.total_probes,
                    });
                  }}
                />
              </g>
            );
          })}

          {/* Hover crosshair guide */}
          {hoveredPoint && (
            <line
              x1={hoveredPoint.x}
              y1={padTop}
              x2={hoveredPoint.x}
              y2={padTop + plotH}
              style={{ stroke: 'var(--text-muted)' }}
              strokeWidth="1"
              strokeDasharray="2 2"
            />
          )}

          {/* X-axis time labels */}
          <text
            x={padLeft}
            y={svgHeight - 6}
            fontSize="9"
            style={{ fill: 'var(--text-muted)' }}
            fontFamily="monospace"
          >
            {new Date(start_ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
          </text>
          <text
            x={svgWidth - padRight}
            y={svgHeight - 6}
            textAnchor="end"
            fontSize="9"
            style={{ fill: 'var(--text-muted)' }}
            fontFamily="monospace"
          >
            {new Date(end_ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
          </text>
        </svg>

        {/* Hover Tooltip Box */}
        {hoveredPoint && (
          <div
            className="absolute z-20 pointer-events-none rounded-md p-2 shadow-lg text-[11px] font-mono tabular-nums border border-[var(--border-hairline)] bg-[var(--bg-panel)]/95 text-[var(--text-primary)]"
            style={{
              left: Math.min(hoveredPoint.x + 12, svgWidth - 180),
              top: Math.max(8, hoveredPoint.y - 45),
            }}
          >
            <div className="text-[var(--text-muted)] text-[10px] border-b border-[var(--border-hairline)] pb-0.5 mb-1">
              {new Date(hoveredPoint.timeMs).toLocaleTimeString()}
            </div>
            <div className="space-y-0.5">
              <div className="flex justify-between space-x-3">
                <span className="text-[var(--text-muted)]">RTT:</span>
                <span className="font-bold text-[var(--text-primary)]">
                  {hoveredPoint.rttMs != null ? `${hoveredPoint.rttMs.toFixed(1)} ms` : '—'}
                </span>
              </div>
              <div className="flex justify-between space-x-3">
                <span className="text-[var(--text-muted)]">Loss:</span>
                <span className={`font-bold ${(hoveredPoint.lossPercent ?? 0) > 0 ? 'text-rose-400' : 'text-emerald-400'}`}>
                  {hoveredPoint.lossPercent?.toFixed(1) ?? '0.0'}%
                </span>
              </div>
              <div className="flex justify-between space-x-3">
                <span className="text-[var(--text-muted)]">Probes:</span>
                <span>{hoveredPoint.totalProbes ?? 1}</span>
              </div>
              {hoveredPoint.ip && (
                <div className="flex justify-between space-x-3 text-[10px] text-[var(--text-muted)]">
                  <span>IP:</span>
                  <span className="font-mono text-[var(--text-primary)]">{hoveredPoint.ip}</span>
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function LiveSparkline({ observations, width = 240, height = 40 }: SparklineProps) {
  if (!observations || observations.length === 0) {
    return (
      <div className="h-10 flex items-center justify-center text-[10px] text-[var(--text-muted)] font-mono">
        Waiting for probes…
      </div>
    );
  }

  // Take latest up to 45 observations
  const data = observations.slice(-45);
  const padding = 3;
  const chartW = width - padding * 2;
  const chartH = height - padding * 2;

  // Find max RTT among successful probes (minimum 10ms ceiling for visual stability)
  const validRtts = data.filter(d => d.success && d.rtt_measured_us != null).map(d => d.rtt_measured_us! / 1000);
  const maxRtt = Math.max(10, ...validRtts);

  // Compute points
  const points: { x: number; y: number; success: boolean; rtt: number | null }[] = [];
  data.forEach((obs, idx) => {
    const x = padding + (idx / Math.max(1, data.length - 1)) * chartW;
    if (obs.success && obs.rtt_measured_us != null) {
      const rtt = obs.rtt_measured_us / 1000;
      const y = padding + chartH - (rtt / maxRtt) * chartH;
      points.push({ x, y, success: true, rtt });
    } else {
      points.push({ x, y: padding + 1, success: false, rtt: null });
    }
  });

  // Build SVG path for successful segments (clean 1.5px stroke, no blurry gradient bloat)
  let linePath = '';
  const successPoints = points.filter(p => p.success);

  if (successPoints.length > 1) {
    linePath = `M ${successPoints[0].x} ${successPoints[0].y}`;
    for (let i = 1; i < successPoints.length; i++) {
      linePath += ` L ${successPoints[i].x} ${successPoints[i].y}`;
    }
  }

  const lossPoints = points.filter(p => !p.success);

  return (
    <div className="relative">
      <svg width="100%" height={height} viewBox={`0 0 ${width} ${height}`} className="overflow-visible">
        {/* Baseline grid line */}
        <line
          x1={padding}
          y1={height - padding}
          x2={width - padding}
          y2={height - padding}
          style={{ stroke: 'var(--border-hairline)' }}
          strokeWidth="1"
        />

        {/* Crisp 1.5px Stroke Line */}
        {linePath && (
          <path
            d={linePath}
            fill="none"
            stroke="#38BDF8"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        )}

        {/* Latest probe indicator dot */}
        {successPoints.length > 0 && (
          <circle
            cx={successPoints[successPoints.length - 1].x}
            cy={successPoints[successPoints.length - 1].y}
            r="2"
            fill="#38BDF8"
          />
        )}

        {/* Packet loss tick marks (sharp crimson vertical ticks) */}
        {lossPoints.map((p, i) => (
          <g key={`loss-${i}`}>
            <line
              x1={p.x}
              y1={padding}
              x2={p.x}
              y2={height - padding}
              stroke="#EF4444"
              strokeWidth="1.5"
              strokeOpacity="0.9"
            />
          </g>
        ))}
      </svg>
      <div className="flex justify-between items-center text-[9px] text-[var(--text-muted)] font-mono tabular-nums mt-0.5 px-0.5">
        <span>-{(data.length * 1.5).toFixed(0)}s</span>
        <span>max {maxRtt.toFixed(0)}ms</span>
        <span>now</span>
      </div>
    </div>
  );
}

function downloadBlob(data: string, mime: string, filename: string) {
  const blob = new Blob([data], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url; a.download = filename; a.click();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}

export default App;
