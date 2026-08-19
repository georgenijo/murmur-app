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

/**
 * Schedule a short output-only cue and return immediately. The generated nodes
 * never share Murmur's Rust microphone/PCM path, and no captured samples are
 * supplied to this function.
 */
export function playSoundCue(cue: SoundCue, volumePercent: number): void {
  const volume = clampCueVolume(volumePercent) / 100;
  if (volume === 0) return;
  const AudioContextType = window.AudioContext
    ?? (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!AudioContextType) return;

  const context = new AudioContextType();
  const startedAt = context.currentTime;
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
  }
  const endMs = Math.max(...CUE_TONES[cue].map((tone) => tone.offsetMs + tone.durationMs));
  window.setTimeout(() => { void context.close(); }, endMs + 30);
}
