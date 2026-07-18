import { describe, expect, it } from 'vitest';
import {
  isHotkeyTapRejectedPayload,
  shouldShowHotkeyMissFeedback,
} from './hotkeyFeedback';

describe('hotkey timing feedback', () => {
  it('shows only opt-in expired-window feedback while idle', () => {
    const miss = { reason: 'second_tap_expired', mode: 'double_tap' };
    expect(shouldShowHotkeyMissFeedback(true, 'idle', miss)).toBe(true);
    expect(shouldShowHotkeyMissFeedback(false, 'idle', miss)).toBe(false);
    expect(shouldShowHotkeyMissFeedback(true, 'recording', miss)).toBe(false);
    expect(shouldShowHotkeyMissFeedback(true, 'processing', miss)).toBe(false);
  });

  it('covers Double-Tap and Both but ignores non-timing rejection reasons', () => {
    expect(shouldShowHotkeyMissFeedback(true, 'idle', {
      reason: 'second_tap_expired',
      mode: 'both',
    })).toBe(true);
    for (const reason of [
      'held_too_long',
      'combo_cancelled',
      'single_short_tap_noop',
      'processing_skipped',
    ]) {
      expect(shouldShowHotkeyMissFeedback(true, 'idle', {
        reason,
        mode: 'double_tap',
      })).toBe(false);
    }
    expect(shouldShowHotkeyMissFeedback(true, 'idle', {
      reason: 'second_tap_expired',
      mode: 'hold_down',
    })).toBe(false);
  });

  it('rejects malformed event payloads', () => {
    expect(isHotkeyTapRejectedPayload(null)).toBe(false);
    expect(isHotkeyTapRejectedPayload({ reason: 'second_tap_expired' })).toBe(false);
    expect(isHotkeyTapRejectedPayload({ reason: 'second_tap_expired', mode: 'both' })).toBe(true);
  });
});
