/**
 * Deterministic trailing-silence detector for hands-free dictation.
 *
 * Double-tap recordings run until you tap again. This reducer watches the same
 * `audio-level` RMS samples the overlay waveform already consumes and reports
 * when a recording has been quiet long enough to finish itself.
 *
 * Two rules keep it from ever cutting someone off:
 *
 * 1. **It must hear speech first.** Until `minSpeechMs` of above-threshold
 *    audio has accumulated, silence is ignored entirely — a recording started
 *    before the user is ready never self-terminates.
 * 2. **The threshold only ever rises above the absolute floor.** It is
 *    `max(MIN_SPEECH_LEVEL, peak * RELATIVE_FLOOR)`, so on a quiet microphone
 *    the detector simply never arms and the feature degrades to today's
 *    behavior instead of stopping early.
 *
 * Pure and time-injected: the caller supplies each sample's timestamp, so the
 * whole state machine is unit-testable without timers.
 */

/** Absolute RMS floor for "this is speech, not room tone". */
export const MIN_SPEECH_LEVEL = 0.015;

/** Speech threshold as a fraction of the loudest sample seen this recording. */
export const RELATIVE_FLOOR = 0.08;

/** Cumulative speech required before trailing silence can trigger a stop. */
export const MIN_SPEECH_MS = 400;

/**
 * Largest gap between two samples that still counts toward speech/silence
 * totals. Levels arrive ~60×/s; a longer gap means the stream stalled or the
 * window was suspended, and charging that whole gap to silence would end the
 * recording for a reason that has nothing to do with the speaker.
 */
export const MAX_SAMPLE_GAP_MS = 500;

export interface SilenceAutoStopState {
  /** Loudest RMS seen this recording, used for the relative threshold. */
  peak: number;
  /** Cumulative above-threshold audio. */
  speechMs: number;
  /** True once `speechMs` has reached `MIN_SPEECH_MS`. */
  armed: boolean;
  /** Trailing silence accumulated since the last speech sample. */
  silenceMs: number;
  /** Timestamp of the previous sample, or null before the first one. */
  lastSampleMs: number | null;
  /** Latched once the stop has been reported, so it fires exactly once. */
  stopped: boolean;
}

export function initialSilenceState(): SilenceAutoStopState {
  return { peak: 0, speechMs: 0, armed: false, silenceMs: 0, lastSampleMs: null, stopped: false };
}

/** The RMS level a sample must reach to count as speech, given what has been
 *  heard so far. */
export function speechThreshold(peak: number): number {
  return Math.max(MIN_SPEECH_LEVEL, peak * RELATIVE_FLOOR);
}

export interface SilenceAutoStopResult {
  state: SilenceAutoStopState;
  /** True on the single transition where the recording should stop. */
  stop: boolean;
}

/**
 * Fold one audio-level sample into the state.
 *
 * @param silenceMsToStop trailing silence that ends the recording; `<= 0`
 *        disables the detector entirely (the state is returned untouched).
 */
export function reduceSilenceSample(
  state: SilenceAutoStopState,
  sample: { level: number; atMs: number },
  silenceMsToStop: number,
): SilenceAutoStopResult {
  if (state.stopped || silenceMsToStop <= 0) return { state, stop: false };

  const level = Number.isFinite(sample.level) ? Math.max(0, sample.level) : 0;
  const peak = Math.max(state.peak, level);
  const elapsed = state.lastSampleMs === null
    ? 0
    : Math.min(Math.max(0, sample.atMs - state.lastSampleMs), MAX_SAMPLE_GAP_MS);

  if (level >= speechThreshold(peak)) {
    const speechMs = state.speechMs + elapsed;
    return {
      state: {
        peak,
        speechMs,
        armed: state.armed || speechMs >= MIN_SPEECH_MS,
        silenceMs: 0,
        lastSampleMs: sample.atMs,
        stopped: false,
      },
      stop: false,
    };
  }

  // Silence only counts once the detector has actually heard the speaker.
  const silenceMs = state.armed ? state.silenceMs + elapsed : 0;
  const stop = state.armed && silenceMs >= silenceMsToStop;
  return {
    state: {
      peak,
      speechMs: state.speechMs,
      armed: state.armed,
      silenceMs,
      lastSampleMs: sample.atMs,
      stopped: stop,
    },
    stop,
  };
}
