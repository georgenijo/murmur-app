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

export function modelUnavailableFromPayload(value: unknown): ModelUnavailableReason | null {
  if (!value || typeof value !== 'object') return null;
  const reason = (value as Record<string, unknown>).reason;
  return typeof reason === 'string' && REASONS.includes(reason)
    ? reason as ModelUnavailableReason
    : null;
}

/** The floating pill is 120pt wide: about 78px of text beside its glyph at
 * 11px semibold, so every word stays within `MODEL_WORD_MAX_CHARS`. */
export const MODEL_WORD_MAX_CHARS = 12;

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

/** The settings destination every model cue opens. */
export const MODEL_SETTINGS_TARGET = { page: 'ai-transcription', target: 'transcription-model' } as const;
