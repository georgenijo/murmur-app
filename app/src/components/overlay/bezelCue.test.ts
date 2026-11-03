import { describe, expect, it } from 'vitest';
import { bezelCue } from './bezelCue';
import type { OverlayIndicator } from './deriveVisual';

describe('bezelCue', () => {
  it('pairs every visible state with one shape and one word', () => {
    const cases: [OverlayIndicator, string, string][] = [
      [{ kind: 'calibrating' }, 'arrows', 'Calibrating'],
      [{ kind: 'cancelled' }, 'cross', 'Cancelled'],
      [{ kind: 'secureField' }, 'warn', 'Secure field'],
      [{ kind: 'microphoneFailure', failure: 'chooseMicrophone' }, 'warn', 'No mic'],
      [{ kind: 'microphoneFailure', failure: 'openMicrophoneSettings' }, 'warn', 'Mic denied'],
      [{ kind: 'microphoneFailure', failure: 'waitForPartialTranscription' }, 'warn', 'Mic lost'],
      [{ kind: 'microphoneFailure', failure: 'retry' }, 'warn', 'Mic failed'],
      [{ kind: 'transformBusy' }, 'warn', 'Busy'],
      [{ kind: 'hotkeyMiss' }, 'warn', 'Tap missed'],
      [{ kind: 'meeting', processing: false }, 'dot', 'Meeting'],
      [{ kind: 'meeting', processing: true }, 'spinner', 'Finishing'],
      [{ kind: 'starting', slow: true }, 'ring', 'Connecting'],
      [{ kind: 'recovering' }, 'spinner', 'Recovering'],
      [{ kind: 'processing' }, 'spinner', 'Transcribing…'],
      [{ kind: 'transforming' }, 'spinner', 'Rewriting'],
    ];
    for (const [indicator, glyph, word] of cases) {
      expect(bezelCue(indicator)).toEqual({ glyph, word });
    }
    // Recording pairs the dot with the waveform instead of a word.
    expect(bezelCue({ kind: 'recording' })).toEqual({ glyph: 'dot', word: '' });
  });

  it('checks a finish and offers the delivery retry as the word', () => {
    expect(bezelCue({ kind: 'idle', dimmed: false }, null, true)).toEqual({ glyph: 'check', word: 'Done' });
    const delivery = (kind: 'auto_pasted' | 'retrying' | 'empty' | 'failed' | 'busy' | 'clipboard_only' | 'confirmed_clipboard') =>
      bezelCue({ kind: 'clipboardOnly' }, { kind, message: 'Synthetic.' });
    expect(delivery('auto_pasted')).toEqual({ glyph: 'check', word: 'Pasted' });
    expect(delivery('retrying')).toEqual({ glyph: 'spinner', word: 'Trying…' });
    expect(delivery('empty')).toEqual({ glyph: 'cross', word: 'Nothing' });
    expect(delivery('failed')).toEqual({ glyph: 'warn', word: 'Try again', retry: true });
    expect(delivery('busy')).toEqual({ glyph: 'warn', word: 'Try again', retry: true });
    expect(delivery('clipboard_only')).toEqual({ glyph: 'check', word: 'Paste', retry: true });
    expect(delivery('confirmed_clipboard')).toEqual({ glyph: 'check', word: 'Paste', retry: true });
  });
});
