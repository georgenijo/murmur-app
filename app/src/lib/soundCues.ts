export type SoundCue = 'start' | 'stop' | 'success' | 'failure';

interface Tone {
  frequency: number;
  offsetMs: number;
  durationMs: number;
}

const CUE_TONES: Record<SoundCue, readonly Tone[]> = {
  start: [{ frequency: 660, offsetMs: 0, durationMs: 70 }],
  stop: [{ frequency: 440, offsetMs: 0, durationMs: 70 }],
  success: [
    { frequency: 587, offsetMs: 0, durationMs: 65 },
    { frequency: 784, offsetMs: 55, durationMs: 90 },
  ],
  failure: [
    { frequency: 330, offsetMs: 0, durationMs: 80 },
    { frequency: 220, offsetMs: 65, durationMs: 120 },
  ],
};

export function clampCueVolume(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return 45;
  return Math.max(0, Math.min(100, Math.round(value)));
}

/** Extra time after a tone ends before a cue counts as played out. */
export const CUE_PLAYOUT_MARGIN_MS = 30;
/** Settle anyway this long after the scheduled end if `onended` never fires. */
const CUE_SETTLE_FALLBACK_MS = 1000;
/** Bound on the output latency added to a cue's play-out time. */
const MAX_OUTPUT_LATENCY_MS = 200;

/** Scheduled length of a cue, from its first tone's start to its last tone's end. */
export function cueDurationMs(cue: SoundCue): number {
  return Math.max(...CUE_TONES[cue].map((tone) => tone.offsetMs + tone.durationMs));
}

/**
 * Schedule a short output-only cue and return at once. The generated nodes
 * never share Murmur's Rust microphone/PCM path, and no captured samples are
 * supplied to this function.
 *
 * The returned promise settles once the cue has played out of the output: its
 * last tone ended on the audio clock, plus the context's output latency and
 * {@link CUE_PLAYOUT_MARGIN_MS}. It settles at once when nothing plays, and
 * never rejects. "Mute speakers while dictating" waits for it.
 */
export function playSoundCue(cue: SoundCue, volumePercent: number): Promise<void> {
  const volume = clampCueVolume(volumePercent) / 100;
  if (volume === 0) return Promise.resolve();
  const AudioContextType = window.AudioContext
    ?? (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!AudioContextType) return Promise.resolve();

  const context = new AudioContextType();
  const startedAt = context.currentTime;
  let last: { node: OscillatorNode; end: number } | null = null;
  for (const tone of CUE_TONES[cue]) {
    const oscillator = context.createOscillator();
    const gain = context.createGain();
    const toneStart = startedAt + tone.offsetMs / 1000;
    const toneEnd = toneStart + tone.durationMs / 1000;
    oscillator.type = 'sine';
    oscillator.frequency.setValueAtTime(tone.frequency, toneStart);
    gain.gain.setValueAtTime(0.0001, toneStart);
    gain.gain.exponentialRampToValueAtTime(Math.max(0.0001, volume * 0.18), toneStart + 0.008);
    gain.gain.exponentialRampToValueAtTime(0.0001, toneEnd);
    oscillator.connect(gain);
    gain.connect(context.destination);
    oscillator.start(toneStart);
    oscillator.stop(toneEnd);
    if (!last || toneEnd > last.end) last = { node: oscillator, end: toneEnd };
  }
  return new Promise<void>((resolve) => {
    let settled = false;
    const settle = () => {
      if (settled) return;
      settled = true;
      void Promise.resolve(context.close()).catch(() => {});
      resolve();
    };
    const fallback = window.setTimeout(settle, cueDurationMs(cue) + CUE_SETTLE_FALLBACK_MS);
    if (last) {
      last.node.onended = () => {
        const latencyMs = ((context.outputLatency || 0) + (context.baseLatency || 0)) * 1000;
        window.clearTimeout(fallback);
        window.setTimeout(settle, Math.min(MAX_OUTPUT_LATENCY_MS, latencyMs) + CUE_PLAYOUT_MARGIN_MS);
      };
    }
  });
}
