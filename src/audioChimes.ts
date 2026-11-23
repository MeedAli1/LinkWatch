/**
 * audioChimes.ts — Custom synthesized audio chimes for LinkWatch.
 *
 * Uses the Web Audio API with soft envelope shaping (attack/decay)
 * so it produces smooth, clean tones without external sound files.
 */

let audioCtx: AudioContext | null = null;

function getAudioContext(): AudioContext {
  if (!audioCtx) {
    const AudioContextClass = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    audioCtx = new AudioContextClass();
  }
  if (audioCtx.state === 'suspended') {
    audioCtx.resume();
  }
  return audioCtx;
}

export type ChimeProfile = 'descending_triad' | 'sonar_ping' | 'marimba_alert' | 'urgent_glitch';

/**
 * Play a custom 3-tone chime profile.
 *
 * @param profile Which harmonic signature to trigger
 * @param volume Master volume 0.0 - 1.0 (default 0.3)
 */
export function playPacketLossChime(profile: ChimeProfile = 'descending_triad', volume: number = 0.3) {
  try {
    const ctx = getAudioContext();
    const now = ctx.currentTime;

    const masterGain = ctx.createGain();
    masterGain.gain.setValueAtTime(Math.max(0.01, Math.min(1.0, volume)), now);
    masterGain.connect(ctx.destination);

    if (profile === 'descending_triad') {
      // 3 descending minor notes (E5 -> C5 -> A4) indicating loss / drop
      const notes = [
        { freq: 659.25, time: 0.00, dur: 0.22, type: 'sine' as OscillatorType }, // E5
        { freq: 523.25, time: 0.12, dur: 0.22, type: 'sine' as OscillatorType }, // C5
        { freq: 440.00, time: 0.24, dur: 0.45, type: 'sine' as OscillatorType }, // A4
      ];

      notes.forEach(({ freq, time, dur, type }) => {
        const osc = ctx.createOscillator();
        const noteGain = ctx.createGain();
        osc.type = type;
        osc.frequency.setValueAtTime(freq, now + time);

        noteGain.gain.setValueAtTime(0.001, now + time);
        noteGain.gain.linearRampToValueAtTime(0.4, now + time + 0.015);
        noteGain.gain.exponentialRampToValueAtTime(0.0001, now + time + dur);

        osc.connect(noteGain);
        noteGain.connect(masterGain);
        osc.start(now + time);
        osc.stop(now + time + dur + 0.05);
      });
    } else if (profile === 'sonar_ping') {
      // 3 subtle sonar radar pulses
      [0.0, 0.14, 0.28].forEach((timeOffset, idx) => {
        const freq = 880 + idx * 110;
        const osc = ctx.createOscillator();
        const noteGain = ctx.createGain();
        osc.type = 'sine';
        osc.frequency.setValueAtTime(freq, now + timeOffset);

        noteGain.gain.setValueAtTime(0.001, now + timeOffset);
        noteGain.gain.linearRampToValueAtTime(0.35, now + timeOffset + 0.01);
        noteGain.gain.exponentialRampToValueAtTime(0.0001, now + timeOffset + 0.18);

        osc.connect(noteGain);
        noteGain.connect(masterGain);
        osc.start(now + timeOffset);
        osc.stop(now + timeOffset + 0.2);
      });
    } else if (profile === 'marimba_alert') {
      // Warm 3-tone marimba chime (A4 -> C#5 -> E5)
      const notes = [440, 554.37, 659.25];
      notes.forEach((freq, idx) => {
        const time = idx * 0.09;
        const osc = ctx.createOscillator();
        const noteGain = ctx.createGain();
        osc.type = 'triangle'; // triangle gives soft wood/marimba timbre
        osc.frequency.setValueAtTime(freq, now + time);

        noteGain.gain.setValueAtTime(0.001, now + time);
        noteGain.gain.linearRampToValueAtTime(0.45, now + time + 0.008);
        noteGain.gain.exponentialRampToValueAtTime(0.0001, now + time + 0.35);

        osc.connect(noteGain);
        noteGain.connect(masterGain);
        osc.start(now + time);
        osc.stop(now + time + 0.4);
      });
    } else {
      // urgent_glitch: 3 fast staccato warning bleeps
      [600, 750, 900].forEach((freq, idx) => {
        const time = idx * 0.07;
        const osc = ctx.createOscillator();
        const noteGain = ctx.createGain();
        osc.type = 'sawtooth';
        osc.frequency.setValueAtTime(freq, now + time);

        noteGain.gain.setValueAtTime(0.001, now + time);
        noteGain.gain.linearRampToValueAtTime(0.2, now + time + 0.005);
        noteGain.gain.exponentialRampToValueAtTime(0.0001, now + time + 0.06);

        osc.connect(noteGain);
        noteGain.connect(masterGain);
        osc.start(now + time);
        osc.stop(now + time + 0.08);
      });
    }
  } catch (err) {
    console.error('Audio chime error:', err);
  }
}
