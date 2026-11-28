import { ASSUMED_TYPING_WPM } from './activityStats';

/**
 * Opt-in typing-speed check for Insights (`typingSpeedWpm`, 0 = not measured,
 * the default). The user types a fixed practice sentence; only the resulting
 * whole-number speed is saved. The typed text is never stored, logged, or sent
 * anywhere, and the check never touches recording, transcription, delivery or
 * history. The saved speed only replaces the assumed 40 wpm in the
 * "Estimated typing time saved" figure.
 */
export const TYPING_SPEED_OFF = 0;
export const TYPING_SPEED_MIN = 5;
export const TYPING_SPEED_MAX = 250;
/** Below this, a run is too short to say anything about speed. */
export const TYPING_CHECK_MIN_CHARS = 40;
export const TYPING_CHECK_MIN_MS = 2000;
/** Below this share of correct characters, the run measures typos, not speed. */
export const TYPING_CHECK_MIN_ACCURACY = 0.8;

/** Plain-ASCII practice sentences so every keyboard layout can type them. */
export const TYPING_CHECK_PASSAGES = [
  'The quick brown fox jumps over the lazy dog while the farmer counts his sheep by the gate.',
  'Please send the final draft to the team before lunch so we can review it this afternoon.',
  'A short walk after dinner helps me think through the problems I could not solve at my desk.',
  'We moved the meeting to Thursday because half of the group will be travelling on Monday.',
] as const;

/** Any finite value is rounded and clamped; zero, negatives and junk are off. */
export function sanitizeTypingSpeedWpm(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return TYPING_SPEED_OFF;
  const rounded = Math.round(value);
  if (rounded <= 0) return TYPING_SPEED_OFF;
  return Math.min(Math.max(rounded, TYPING_SPEED_MIN), TYPING_SPEED_MAX);
}

/** The speed the time-saved estimate should use: the measured one, else the assumption. */
export function effectiveTypingWpm(setting: number): number {
  const measured = sanitizeTypingSpeedWpm(setting);
  return measured === TYPING_SPEED_OFF ? ASSUMED_TYPING_WPM : measured;
}

export type TypingCheckResult =
  | { ok: true; wpm: number; accuracy: number; correctChars: number; typedChars: number }
  | { ok: false; reason: 'too-short' | 'too-many-typos'; accuracy: number; typedChars: number };

/**
 * Net words per minute: characters typed correctly at their position, divided
 * by five (the standard word length), per minute of elapsed time.
 */
export function scoreTypingCheck(passage: string, typed: string, elapsedMs: number): TypingCheckResult {
  const typedChars = typed.length;
  let correctChars = 0;
  for (let index = 0; index < typedChars && index < passage.length; index++) {
    if (typed[index] === passage[index]) correctChars++;
  }
  const accuracy = typedChars > 0 ? correctChars / typedChars : 0;
  if (typedChars < TYPING_CHECK_MIN_CHARS || !Number.isFinite(elapsedMs) || elapsedMs < TYPING_CHECK_MIN_MS) {
    return { ok: false, reason: 'too-short', accuracy, typedChars };
  }
  if (accuracy < TYPING_CHECK_MIN_ACCURACY) {
    return { ok: false, reason: 'too-many-typos', accuracy, typedChars };
  }
  const wpm = sanitizeTypingSpeedWpm((correctChars / 5) / (elapsedMs / 60_000));
  return { ok: true, wpm: Math.max(wpm, TYPING_SPEED_MIN), accuracy, correctChars, typedChars };
}

/** How many times faster dictation is than typing, to one decimal; null when either is unknown. */
export function dictationSpeedup(dictationWpm: number, typingWpm: number): number | null {
  if (!(dictationWpm > 0) || !(typingWpm > 0)) return null;
  return Math.round((dictationWpm / typingWpm) * 10) / 10;
}
