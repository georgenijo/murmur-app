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
  return { kind: 'idle', dimmed: disabled };
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

  it('dims the idle mic icon only when disabled and otherwise idle', () => {
    expect(deriveVisual('idle', false, false, true).indicator).toEqual({ kind: 'idle', dimmed: true });
    expect(deriveVisual('idle', false, false, false).indicator).toEqual({ kind: 'idle', dimmed: false });
  });
});
