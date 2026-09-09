import { describe, expect, it } from 'vitest';
import { median, percentile } from './numeric';

describe('percentile', () => {
  it('returns 0 for an empty array', () => {
    expect(percentile([], 0.5)).toBe(0);
  });

  it('returns the single value for a one-element array regardless of fraction', () => {
    expect(percentile([42], 0)).toBe(42);
    expect(percentile([42], 0.5)).toBe(42);
    expect(percentile([42], 1)).toBe(42);
  });

  it('sorts unsorted input before computing the percentile', () => {
    expect(percentile([5, 1, 3, 2, 4], 0.5)).toBe(3);
  });

  it('computes nearest-rank percentiles', () => {
    const values = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
    expect(percentile(values, 0.5)).toBe(50);
    expect(percentile(values, 0.95)).toBe(100);
    expect(percentile(values, 0)).toBe(10);
    expect(percentile(values, 1)).toBe(100);
  });

  it('does not mutate the input array', () => {
    const values = [5, 1, 3];
    const copy = [...values];
    percentile(values, 0.5);
    expect(values).toEqual(copy);
  });
});

describe('median', () => {
  it('returns null for an empty array', () => {
    expect(median([])).toBeNull();
  });

  it('returns the single value for a one-element array', () => {
    expect(median([7])).toBe(7);
  });

  it('averages the two middle values for an even-length array', () => {
    expect(median([1, 2, 3, 4])).toBe(2.5);
  });

  it('returns the middle value for an odd-length array', () => {
    expect(median([1, 2, 3, 4, 5])).toBe(3);
  });

  it('sorts unsorted input before computing the median', () => {
    expect(median([5, 1, 4, 2, 3])).toBe(3);
    expect(median([40, 10, 30, 20])).toBe(25);
  });

  it('does not mutate the input array', () => {
    const values = [5, 1, 3];
    const copy = [...values];
    median(values);
    expect(values).toEqual(copy);
  });
});
