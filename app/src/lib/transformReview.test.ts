import { describe, expect, it } from 'vitest';
import {
  isPopoverBox,
  isTransformReviewContent,
  isTransformStateChangedEvent,
  normalizeReviewErrorCode,
} from './transformReview';

describe('isTransformStateChangedEvent', () => {
  it('accepts a bare state with no errorCode', () => {
    expect(isTransformStateChangedEvent({ state: 'listening' })).toBe(true);
  });

  it('accepts a state with a known errorCode', () => {
    expect(isTransformStateChangedEvent({ state: 'failed', errorCode: 'timeout' })).toBe(true);
  });

  it('accepts a state with an unrecognized errorCode instead of rejecting the whole event', () => {
    // Regression: an unknown errorCode used to invalidate the entire event,
    // leaving the popover stuck rendering its prior state forever.
    expect(isTransformStateChangedEvent({ state: 'failed', errorCode: 'some_future_code' })).toBe(true);
  });

  it('rejects a payload with an invalid state', () => {
    expect(isTransformStateChangedEvent({ state: 'not_a_state' })).toBe(false);
  });

  it('rejects non-object payloads', () => {
    expect(isTransformStateChangedEvent(null)).toBe(false);
    expect(isTransformStateChangedEvent('failed')).toBe(false);
    expect(isTransformStateChangedEvent(undefined)).toBe(false);
  });
});

describe('normalizeReviewErrorCode', () => {
  it('passes known error codes through unchanged', () => {
    expect(normalizeReviewErrorCode('timeout')).toBe('timeout');
    expect(normalizeReviewErrorCode('model_not_downloaded')).toBe('model_not_downloaded');
  });

  it('recognizes undo-path apply error codes (item 12)', () => {
    // The undo-failure UX re-emits `applied` carrying these codes; they must
    // normalize through so a real undo error is not coerced to null.
    expect(normalizeReviewErrorCode('clipboard_unavailable')).toBe('clipboard_unavailable');
    expect(normalizeReviewErrorCode('paste_failed')).toBe('paste_failed');
    expect(normalizeReviewErrorCode('not_applied')).toBe('not_applied');
  });

  it('coerces unknown or missing error codes to null', () => {
    expect(normalizeReviewErrorCode('some_future_code')).toBeNull();
    expect(normalizeReviewErrorCode(undefined)).toBeNull();
    expect(normalizeReviewErrorCode(null)).toBeNull();
    expect(normalizeReviewErrorCode(42)).toBeNull();
  });
});

describe('isTransformReviewContent', () => {
  it('accepts a well-formed content payload', () => {
    expect(
      isTransformReviewContent({ instruction: 'a', original: 'b', proposed: 'c' }),
    ).toBe(true);
  });

  it('rejects a payload missing a required field', () => {
    expect(isTransformReviewContent({ instruction: 'a', original: 'b' })).toBe(false);
  });
});

describe('isPopoverBox', () => {
  it('accepts a well-formed applied-surface box', () => {
    expect(isPopoverBox({ x: 0, y: 25, width: 320, height: 76, flipped: false })).toBe(true);
  });

  it('rejects a payload with a wrong-typed or missing field', () => {
    expect(isPopoverBox({ x: 0, y: 25, width: 320, height: 76, flipped: 'no' })).toBe(false);
    expect(isPopoverBox({ x: 0, y: 25, width: 320, height: 76 })).toBe(false);
    expect(isPopoverBox(null)).toBe(false);
  });
});
