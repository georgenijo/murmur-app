import { describe, expect, it } from 'vitest';
import {
  isHiddenPayload,
  isQueryStatePayload,
  isValidPassId,
  QUERY_STATES,
} from './queryReview';

describe('isValidPassId', () => {
  it('accepts positive safe integers', () => {
    expect(isValidPassId(1)).toBe(true);
    expect(isValidPassId(42)).toBe(true);
  });

  it('rejects zero, negative, non-integer, non-safe-integer, and non-number values', () => {
    expect(isValidPassId(0)).toBe(false);
    expect(isValidPassId(-1)).toBe(false);
    expect(isValidPassId(1.5)).toBe(false);
    expect(isValidPassId(Number.MAX_SAFE_INTEGER + 1)).toBe(false);
    expect(isValidPassId('1')).toBe(false);
    expect(isValidPassId(null)).toBe(false);
    expect(isValidPassId(undefined)).toBe(false);
  });
});

describe('isQueryStatePayload', () => {
  it('accepts a well-formed payload for every known state', () => {
    for (const state of QUERY_STATES) {
      expect(isQueryStatePayload({ queryPassId: 1, state, errorCode: null })).toBe(true);
    }
  });

  it('accepts a payload with a string errorCode and an optional usage field', () => {
    expect(isQueryStatePayload({
      queryPassId: 1,
      state: 'failed',
      errorCode: 'timed_out',
      usage: { anything: true },
    })).toBe(true);
  });

  it('rejects a non-object, null, an array, or a missing/invalid queryPassId', () => {
    expect(isQueryStatePayload(null)).toBe(false);
    expect(isQueryStatePayload(undefined)).toBe(false);
    expect(isQueryStatePayload('nope')).toBe(false);
    expect(isQueryStatePayload({ state: 'idle', errorCode: null })).toBe(false);
    expect(isQueryStatePayload({ queryPassId: 0, state: 'idle', errorCode: null })).toBe(false);
    expect(isQueryStatePayload({ queryPassId: -1, state: 'idle', errorCode: null })).toBe(false);
  });

  it('rejects an unknown state value', () => {
    expect(isQueryStatePayload({ queryPassId: 1, state: 'bogus', errorCode: null })).toBe(false);
  });

  it('rejects a non-null, non-string errorCode', () => {
    expect(isQueryStatePayload({ queryPassId: 1, state: 'idle', errorCode: 7 })).toBe(false);
  });
});

describe('isHiddenPayload', () => {
  it('accepts a payload with exactly one valid queryPassId field', () => {
    expect(isHiddenPayload({ queryPassId: 1 })).toBe(true);
  });

  it('rejects extra fields, missing fields, or an invalid queryPassId', () => {
    expect(isHiddenPayload({ queryPassId: 1, extra: true })).toBe(false);
    expect(isHiddenPayload({})).toBe(false);
    expect(isHiddenPayload({ queryPassId: 0 })).toBe(false);
    expect(isHiddenPayload({ queryPassId: '1' })).toBe(false);
    expect(isHiddenPayload(null)).toBe(false);
    expect(isHiddenPayload('nope')).toBe(false);
  });
});
