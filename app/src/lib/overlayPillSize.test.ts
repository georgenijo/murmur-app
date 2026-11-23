import { describe, expect, it } from 'vitest';
import { overlayPillSize } from './overlayGeometry';

describe('overlayPillSize', () => {
  it('gives the notch no size at all', () => {
    expect(overlayPillSize('notch', false, true)).toBeUndefined();
    expect(overlayPillSize('notch', true, true)).toBeUndefined();
  });

  it('warning beats words beats compact on the floating pill', () => {
    expect(overlayPillSize('floating_pill', true, true)).toBe('warning');
    expect(overlayPillSize('floating_pill', true, false)).toBe('warning');
    expect(overlayPillSize('floating_pill', false, true)).toBe('words');
    expect(overlayPillSize('floating_pill', false, false)).toBe('compact');
  });
});
