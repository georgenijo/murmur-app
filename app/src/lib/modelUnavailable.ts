/**
 * The selected transcription model cannot serve a dictation. Rust emits
 * `dictation-model-unavailable` when a start is refused up front
 * (`phase: 'start'`) or when preparation fails while transcribing
 * (`phase: 'transcribe'`). The payload is content-free.
 */
export type ModelUnavailableReason =
  | 'notInstalled'
  | 'incomplete'
  | 'downloading'
  | 'unsupported'
  | 'loadFailed';

export const MODEL_UNAVAILABLE_EVENT = 'dictation-model-unavailable';

/** How long the overlay keeps the cue after the last signal. */
export const MODEL_UNAVAILABLE_FLASH_MS = 8000;

const REASONS: readonly string[] = ['notInstalled', 'incomplete', 'downloading', 'unsupported', 'loadFailed'];

export interface ModelUnavailableCue {
  reason: ModelUnavailableReason;
  /** The selected model's catalogue id (a setting); null when the payload has none. */
  model: string | null;
}

export function modelUnavailableFromPayload(value: unknown): ModelUnavailableCue | null {
  if (!value || typeof value !== 'object') return null;
  const { reason, model } = value as Record<string, unknown>;
  if (typeof reason !== 'string' || !REASONS.includes(reason)) return null;
  return { reason: reason as ModelUnavailableReason, model: typeof model === 'string' ? model : null };
}

/** The notch status line has room for one short word, so every word stays
 * within `MODEL_WORD_MAX_CHARS`. The floating pill widens for a full phrase
 * instead (`modelUnavailablePhrase`). */
export const MODEL_WORD_MAX_CHARS = 12;

/** What the widened floating pill says: a clear phrase and the action a click
 * takes. A click always opens Speech-to-text settings, which is where each of
 * these is repaired or watched. */
export function modelUnavailablePhrase(reason: ModelUnavailableReason): { phrase: string; action: string } {
  switch (reason) {
    case 'notInstalled': return { phrase: 'No speech model', action: 'Fix' };
    case 'incomplete': return { phrase: 'Model incomplete', action: 'Fix' };
    case 'downloading': return { phrase: 'Model downloading', action: 'View' };
    case 'unsupported': return { phrase: 'Model unsupported', action: 'Fix' };
    case 'loadFailed': return { phrase: 'Model failed to load', action: 'Fix' };
  }
}

/** One short word for the floating pill and the notch status line. */
export function modelUnavailableWord(reason: ModelUnavailableReason): string {
  switch (reason) {
    case 'notInstalled': return 'No model';
    case 'incomplete': return 'Incomplete';
    case 'downloading': return 'Downloading';
    case 'unsupported': return 'Unsupported';
    case 'loadFailed': return 'Load failed';
  }
}

export function modelUnavailableLabel(reason: ModelUnavailableReason): string {
  return reason === 'downloading'
    ? 'The speech model is still downloading. Try again when it finishes.'
    : reason === 'unsupported'
      ? 'The selected speech model is not supported on this Mac. Click to choose another.'
      : reason === 'loadFailed'
        ? 'The speech model could not be loaded. Click to open Speech-to-text settings.'
        : 'The selected speech model is not ready. Click to open Speech-to-text settings and repair it.';
}

/**
 * Whether a runtime snapshot of the cue's own model shows the problem fixed.
 * For the on-disk problems that is "installed". A `loadFailed` cue is raised
 * precisely while the model IS installed but fails to load, so installed proves
 * nothing there: only a successful load (`ready`) does.
 */
export function modelCueRepaired(
  cue: { reason: ModelUnavailableReason },
  snapshot: { installState: string; lifecycleState?: string },
): boolean {
  return cue.reason === 'loadFailed'
    ? snapshot.lifecycleState === 'ready'
    : snapshot.installState === 'installed';
}

/** The settings destination every model cue opens. */
export const MODEL_SETTINGS_TARGET = { page: 'ai-transcription', target: 'transcription-model' } as const;
