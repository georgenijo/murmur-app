import { describe, expect, it } from 'vitest';
import {
  isFiniteNumber,
  isNonNegativeInteger,
  isPositiveInteger,
  isRecord,
} from './typeGuards';

describe('isRecord', () => {
  it('accepts plain objects', () => {
    expect(isRecord({})).toBe(true);
    expect(isRecord({ a: 1 })).toBe(true);
  });

  it('rejects null, arrays, and non-objects', () => {
    expect(isRecord(null)).toBe(false);
    expect(isRecord(undefined)).toBe(false);
    expect(isRecord([])).toBe(false);
    expect(isRecord([1, 2])).toBe(false);
    expect(isRecord('string')).toBe(false);
    expect(isRecord(42)).toBe(false);
    expect(isRecord(true)).toBe(false);
  });
});

describe('isFiniteNumber', () => {
  it('accepts finite numbers, including negative and zero', () => {
    expect(isFiniteNumber(0)).toBe(true);
    expect(isFiniteNumber(-5)).toBe(true);
    expect(isFiniteNumber(1.5)).toBe(true);
  });

  it('rejects NaN, Infinity, and non-numbers', () => {
    expect(isFiniteNumber(NaN)).toBe(false);
    expect(isFiniteNumber(Infinity)).toBe(false);
    expect(isFiniteNumber(-Infinity)).toBe(false);
    expect(isFiniteNumber('1')).toBe(false);
    expect(isFiniteNumber(null)).toBe(false);
    expect(isFiniteNumber(undefined)).toBe(false);
  });
});

describe('isNonNegativeInteger', () => {
  it('accepts zero and positive safe integers', () => {
    expect(isNonNegativeInteger(0)).toBe(true);
    expect(isNonNegativeInteger(42)).toBe(true);
    expect(isNonNegativeInteger(Number.MAX_SAFE_INTEGER)).toBe(true);
  });

  it('rejects negative numbers, non-integers, unsafe integers, and non-numbers', () => {
    expect(isNonNegativeInteger(-1)).toBe(false);
    expect(isNonNegativeInteger(1.5)).toBe(false);
    expect(isNonNegativeInteger(Number.MAX_SAFE_INTEGER + 1)).toBe(false);
    expect(isNonNegativeInteger('0')).toBe(false);
    expect(isNonNegativeInteger(null)).toBe(false);
    expect(isNonNegativeInteger(NaN)).toBe(false);
  });
});

describe('isPositiveInteger', () => {
  it('accepts positive safe integers', () => {
    expect(isPositiveInteger(1)).toBe(true);
    expect(isPositiveInteger(42)).toBe(true);
  });

  it('rejects zero, negative numbers, non-integers, and non-numbers', () => {
    expect(isPositiveInteger(0)).toBe(false);
    expect(isPositiveInteger(-1)).toBe(false);
    expect(isPositiveInteger(1.5)).toBe(false);
    expect(isPositiveInteger('1')).toBe(false);
    expect(isPositiveInteger(null)).toBe(false);
  });
});
