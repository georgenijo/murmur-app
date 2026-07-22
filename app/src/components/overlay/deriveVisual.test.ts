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

  // Transform-busy refusal flash (issue #329).
  it('transform-busy flash beats hotkey-miss and active statuses', () => {
    const visual = deriveVisual('recording', false, true, false, false, false, true);
    expect(visual.indicator).toEqual({ kind: 'transformBusy' });
  });

  it('secure-field flash beats transform-busy', () => {
    const visual = deriveVisual('idle', false, false, false, false, true, true);
    expect(visual.indicator).toEqual({ kind: 'secureField' });
  });

  it('cancelled beats transform-busy', () => {
    const visual = deriveVisual('idle', true, false, false, false, false, true);
    expect(visual.indicator).toEqual({ kind: 'cancelled' });
  });

  it('transform-busy default-off leaves existing call sites unchanged', () => {
    const visual = deriveVisual('idle', false, false, false);
    expect(visual.indicator).toEqual({ kind: 'idle', dimmed: false });
  });

  // Issue #339 gap 1: cancelled and secure-field co-asserted. Reordering
  // secureField above cancelled must fail this test.
  it('cancelled beats secure-field when both flash at once', () => {
    const visual = deriveVisual('idle', true, false, false, false, true, false);
    expect(visual.indicator).toEqual({ kind: 'cancelled' });

    const everythingOn = deriveVisual('recording', true, true, true, true, true, true);
    expect(everythingOn.indicator).toEqual({ kind: 'cancelled' });
  });

  // Issue #339 gap 2: pin `transforming` into the chain — below recording and
  // processing, above idle. Deleting the branch or hoisting it above an active
  // status must fail one of these.
  it('transforming shows while otherwise idle', () => {
    const visual = deriveVisual('idle', false, false, false, true);
    expect(visual.indicator).toEqual({ kind: 'transforming' });
  });

  it('transforming loses to recording and processing', () => {
    expect(deriveVisual('recording', false, false, false, true).indicator).toEqual({
      kind: 'recording',
    });
    expect(deriveVisual('processing', false, false, false, true).indicator).toEqual({
      kind: 'processing',
    });
  });

  it('transforming loses to every flash above it in the chain', () => {
    expect(deriveVisual('idle', true, false, false, true).indicator).toEqual({ kind: 'cancelled' });
    expect(deriveVisual('idle', false, false, false, true, true).indicator).toEqual({
      kind: 'secureField',
    });
    expect(deriveVisual('idle', false, false, false, true, false, true).indicator).toEqual({
      kind: 'transformBusy',
    });
    expect(deriveVisual('idle', false, true, false, true).indicator).toEqual({
      kind: 'hotkeyMiss',
    });
  });

  it('transforming does not dim, show the tap-missed label, or show the waveform', () => {
    const visual = deriveVisual('idle', false, false, true, true);
    expect(visual.indicator).toEqual({ kind: 'transforming' });
    expect(visual.showTapMissedLabel).toBe(false);
    expect(visual.waveformVisible).toBe(false);
  });
});
