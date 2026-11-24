import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { playPacketLossChime, ChimeProfile } from './audioChimes';

export type OverlayCorner = 'top_left' | 'top_right' | 'bottom_left' | 'bottom_right';

export interface OverlayConfig {
  corner: OverlayCorner;
  font_size: number;
  color: string;
  opacity: number;
  margin: number;
}

interface ProbeObservation {
  rtt_measured_us: number | null;
  success: boolean;
}

interface TargetStat {
  loss_percent: number;
  target_label: string;
}

interface DiagnosticFinding {
  quality_grade: string;
  jitter_ms: number;
  status: string;
}

interface ThroughputData {
  download_mbps: number;
  upload_mbps: number;
  download_kbps: number;
  upload_kbps: number;
}

interface AppStateSnapshot {
  is_monitoring: boolean;
  latest_observations: ProbeObservation[];
  stats: TargetStat[];
  diagnostic: DiagnosticFinding;
}

export default function Overlay() {
  const [snapshot, setSnapshot] = useState<AppStateSnapshot | null>(null);
  const [throughput, setThroughput] = useState<ThroughputData | null>(null);
  const [alertState, setAlertState] = useState<'normal' | 'warn' | 'crit'>('normal');
  const alertTimeoutRef = useRef<number | null>(null);
  const lastChimeTimeRef = useRef<number>(0);

  const [cfg, setCfg] = useState<OverlayConfig>({
    corner: 'top_right',
    font_size: 13,
    color: '#38BDF8',
    opacity: 0.95,
    margin: 16,
  });

  useEffect(() => {
    invoke<OverlayConfig>('get_overlay_config')
      .then(setCfg)
      .catch(() => {});

    const unlisten = listen<OverlayConfig>('overlay-config-updated', (event) => {
      setCfg(event.payload);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const tick = async () => {
      try {
        const snap = await invoke<AppStateSnapshot>('get_snapshot');
        setSnapshot(snap);
        const tp = await invoke<ThroughputData>('get_adapter_throughput');
        setThroughput(tp);

        // Netcode pulse alert logic.
        // Critical must track the current link state, not lifetime history:
        // stats[].loss_percent is a whole-retention aggregate, so a single
        // packet dropped days ago kept the HUD pinned red and masked the
        // configured accent color indefinitely.
        const obs = snap?.latest_observations?.slice(-1)[0];
        const rtt = obs?.rtt_measured_us != null ? obs.rtt_measured_us / 1000 : 0;
        const jitVal = snap?.diagnostic?.jitter_ms ?? 0;

        if (obs && !obs.success) {
          setAlertState('crit');
          if (alertTimeoutRef.current) clearTimeout(alertTimeoutRef.current);
          alertTimeoutRef.current = window.setTimeout(() => setAlertState('normal'), 2500);

          // Trigger 3-tone audio chime (cooldown 3.5s to avoid spamming)
          const nowMs = Date.now();
          if (nowMs - lastChimeTimeRef.current > 3500) {
            lastChimeTimeRef.current = nowMs;
            const profile = (localStorage.getItem('linkwatch_chime_profile') as ChimeProfile) || 'descending_triad';
            const soundEnabled = localStorage.getItem('linkwatch_chime_enabled') === 'true';
            if (soundEnabled) {
              playPacketLossChime(profile, 0.35);
            }
          }
        } else if (rtt > 90 || jitVal > 25) {
          setAlertState('warn');
          if (alertTimeoutRef.current) clearTimeout(alertTimeoutRef.current);
          alertTimeoutRef.current = window.setTimeout(() => setAlertState('normal'), 2500);
        }
      } catch { /* window closed */ }
    };

    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, []);

  const latestObs = snapshot?.latest_observations?.slice(-1)[0];
  const rttMs = latestObs?.rtt_measured_us != null
    ? (latestObs.rtt_measured_us / 1000).toFixed(0)
    : '—';
  const jitter = snapshot?.diagnostic?.jitter_ms?.toFixed(0) ?? '0';
  const grade = snapshot?.diagnostic?.quality_grade ?? '—';
  const loss = snapshot?.stats?.[0]?.loss_percent ?? 0;

  const fmtSpeed = (mbps: number, kbps: number) =>
    mbps >= 1 ? `${mbps.toFixed(0)}M` : `${kbps.toFixed(0)}K`;

  const isRight = cfg.corner === 'top_right' || cfg.corner === 'bottom_right';
  const fontSize = cfg.font_size || 14;

  // Clean native high-contrast colors (no blurry glowing neon slop)
  const activeColor =
    alertState === 'crit'
      ? '#EF4444'
      : alertState === 'warn'
      ? '#F59E0B'
      : cfg.color || '#38BDF8';

  return (
    <div
      style={{
        width: '100vw',
        height: '100vh',
        background: 'transparent',
        backgroundColor: 'transparent',
        display: 'flex',
        alignItems: 'center',
        justifyContent: isRight ? 'flex-end' : 'flex-start',
        padding: '0 8px',
        boxSizing: 'border-box',
        overflow: 'hidden',
        pointerEvents: 'none',
        userSelect: 'none',
        opacity: alertState !== 'normal' ? 1.0 : (cfg.opacity ?? 0.95),
        transition: 'opacity 0.15s ease',
      }}
    >
      <div
        style={{
          display: 'inline-flex',
          alignItems: 'baseline',
          gap: '10px',
          fontFamily: '"JetBrains Mono", "Cascadia Code", Consolas, "SF Mono", monospace',
          fontVariantNumeric: 'tabular-nums',
          fontSize: `${fontSize}px`,
          fontWeight: 700,
          color: activeColor,
          lineHeight: 1,
          letterSpacing: '-0.02em',
          textShadow: `
            -1px -1px 0 #000,
             1px -1px 0 #000,
            -1px  1px 0 #000,
             1px  1px 0 #000,
             0 2px 3px rgba(0,0,0,0.95)
          `,
          whiteSpace: 'nowrap',
          transition: 'color 0.12s ease',
        }}
      >
        {/* Pulse indicator symbol when alert active */}
        {alertState === 'crit' && (
          <span style={{ color: '#EF4444', fontWeight: 800 }}>
            [DROP]
          </span>
        )}
        {alertState === 'warn' && (
          <span style={{ color: '#F59E0B', fontWeight: 800 }}>
            [SPIKE]
          </span>
        )}

        {/* Precision native telemetry readout */}
        <span>{rttMs} ms</span>
        <span style={{ opacity: 0.75, color: '#94a3b8' }}>±{jitter}ms</span>
        <span style={{ color: loss > 0 ? '#EF4444' : activeColor }}>{loss.toFixed(0)}% loss</span>
        <span style={{ opacity: 0.85, color: '#94a3b8' }}>[{grade}]</span>
        {throughput && (
          <span style={{ fontSize: `${Math.max(10, Math.round(fontSize * 0.85))}px`, opacity: 0.85, color: '#94a3b8' }}>
            ↓{fmtSpeed(throughput.download_mbps, throughput.download_kbps)} ↑{fmtSpeed(throughput.upload_mbps, throughput.upload_kbps)}
          </span>
        )}
      </div>
    </div>
  );
}
