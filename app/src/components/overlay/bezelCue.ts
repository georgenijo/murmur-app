import type { OverlayDeliveryCue } from '../../lib/hooks/useOverlayRuntime';
import type { OverlayIndicator } from './deriveVisual';

/** The floating pill pairs every state with one shape and one word: dot =
 * recording, ring = connecting, spinner = working, check = done, cross =
 * cancelled or nothing, triangle = needs attention. */
export type BezelGlyph = 'dot' | 'ring' | 'spinner' | 'check' | 'cross' | 'warn' | 'arrows' | 'mic';

export interface BezelCue {
  glyph: BezelGlyph;
  /** Shown beside the glyph. Empty only for the recording dot (waveform) and
   * the idle mic, which is never visible (the idle pill is hidden). */
  word: string;
  /** Present when the word is a delivery retry button. */
  retry?: boolean;
}

const MICROPHONE_WORDS = {
  chooseMicrophone: 'No mic',
  openMicrophoneSettings: 'Mic denied',
  waitForPartialTranscription: 'Mic lost',
} as const;

export function bezelCue(
  indicator: OverlayIndicator,
  deliveryCue: OverlayDeliveryCue | null = null,
  done = false,
): BezelCue {
  switch (indicator.kind) {
    case 'calibrating': return { glyph: 'arrows', word: 'Calibrating' };
    case 'cancelled': return { glyph: 'cross', word: 'Cancelled' };
    case 'microphoneFailure':
      return {
        glyph: 'warn',
        word: indicator.failure in MICROPHONE_WORDS
          ? MICROPHONE_WORDS[indicator.failure as keyof typeof MICROPHONE_WORDS]
          : 'Mic failed',
      };
    case 'hotkeyMiss': return { glyph: 'warn', word: 'Tap missed' };
    case 'meeting':
      return indicator.processing ? { glyph: 'spinner', word: 'Finishing' } : { glyph: 'dot', word: 'Meeting' };
    case 'starting': return { glyph: 'ring', word: 'Connecting' };
    case 'recording': return { glyph: 'dot', word: '' };
    case 'recovering': return { glyph: 'spinner', word: 'Recovering' };
    case 'processing': return { glyph: 'spinner', word: 'Transcribing…' };
    case 'clipboardOnly':
      switch (deliveryCue?.kind) {
        case 'auto_pasted': return { glyph: 'check', word: 'Pasted' };
        case 'retrying': return { glyph: 'spinner', word: 'Trying…' };
        case 'empty': return { glyph: 'cross', word: 'Nothing' };
        case 'failed':
        case 'busy': return { glyph: 'warn', word: 'Try again', retry: true };
        // Copied: the retry pastes it.
        default: return { glyph: 'check', word: 'Paste', retry: true };
      }
    case 'idle': return done ? { glyph: 'check', word: 'Done' } : { glyph: 'mic', word: '' };
  }
}
