import { describe, expect, it } from 'vitest';
import { deriveVisual, type OverlayIndicator } from './deriveVisual';
import type { DictationStatus } from '../../lib/types';

const STATUSES: DictationStatus[] = ['idle', 'recording', 'processing'];
const BOOLS = [false, true];

/**
 * Independently encodes the expected priority (cancelled > hotkey-miss >
 * recording > processing > idle) so the test does not just re-derive the
 * same branches as the implementation under test.
 */
function expectedIndicator(
  status: DictationStatus,
  showCancelled: boolean,
  showHotkeyMiss: boolean,
  disabled: boolean,
): OverlayIndicator {
  if (showCancelled) return { kind: 'cancelled' };
  if (showHotkeyMiss) return { kind: 'hotkeyMiss' };
  if (status === 'recording') return { kind: 'recording' };
  if (status === 'processing') return { kind: 'processing' };
  return disabled ? { kind: 'disabled' } : { kind: 'idle' };
}

describe('deriveVisual', () => {
  for (const status of STATUSES) {
    for (const showCancelled of BOOLS) {
      for (const showHotkeyMiss of BOOLS) {
        for (const disabled of BOOLS) {
          it(`status=${status} cancelled=${showCancelled} hotkeyMiss=${showHotkeyMiss} disabled=${disabled}`, () => {
            const visual = deriveVisual(status, showCancelled, showHotkeyMiss, disabled);

            expect(visual.indicator).toEqual(
              expectedIndicator(status, showCancelled, showHotkeyMiss, disabled),
            );
            expect(visual.showTapMissedLabel).toBe(showHotkeyMiss);
            expect(visual.waveformVisible).toBe(status === 'recording');
            expect(visual.isActive).toBe(
              status === 'recording' || status === 'processing' || showCancelled || showHotkeyMiss,
            );
          });
        }
      }
    }
  }

  it('locks the priority order explicitly: cancelled beats everything', () => {
    const visual = deriveVisual('recording', true, true, true);
    expect(visual.indicator).toEqual({ kind: 'cancelled' });
  });

  it('locks the priority order explicitly: hotkey-miss beats an active status', () => {
    const visual = deriveVisual('processing', false, true, false);
    expect(visual.indicator).toEqual({ kind: 'hotkeyMiss' });
  });

  it('surfaces global-disable as its own indicator, not a dimmed idle mic', () => {
    expect(deriveVisual('idle', false, false, true).indicator).toEqual({ kind: 'disabled' });
    expect(deriveVisual('idle', false, false, false).indicator).toEqual({ kind: 'idle' });
  });

  // Regression: global-disable was previously signalled only by dropping the
  // idle mic to 15% opacity, which is ~6% effective white on the dark pill and
  // indistinguishable from enabled at a glance. A user clicked the notch ~20
  // times over 20 minutes with no feedback while every start_native_recording
  // was rejected with "app disabled — ignoring". The off state must be a
  // distinct indicator kind so it can render a distinct shape, not an alpha.
  it('does not represent disabled as an opacity variant of idle', () => {
    const off = deriveVisual('idle', false, false, true).indicator;
    expect(off.kind).not.toBe('idle');
    expect(off).not.toHaveProperty('dimmed');
  });

  it('keeps transient flashes and active statuses ahead of disabled', () => {
    expect(deriveVisual('idle', true, false, true).indicator).toEqual({ kind: 'cancelled' });
    expect(deriveVisual('idle', false, true, true).indicator).toEqual({ kind: 'hotkeyMiss' });
    expect(deriveVisual('recording', false, false, true).indicator).toEqual({ kind: 'recording' });
    expect(deriveVisual('processing', false, false, true).indicator).toEqual({ kind: 'processing' });
  });
});
