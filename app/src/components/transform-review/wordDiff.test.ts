import { describe, expect, it } from 'vitest';
import { computeWordDiff, pickDiffLayout } from './wordDiff';

describe('computeWordDiff', () => {
  it('returns an empty diff for two empty strings', () => {
    expect(computeWordDiff('', '')).toEqual([]);
  });

  it('marks everything as added when original is empty', () => {
    const diff = computeWordDiff('', 'hello world');
    expect(diff.every((t) => t.kind === 'added')).toBe(true);
    expect(diff.map((t) => t.text).join('')).toBe('hello world');
  });

  it('marks everything as removed when proposed is empty', () => {
    const diff = computeWordDiff('hello world', '');
    expect(diff.every((t) => t.kind === 'removed')).toBe(true);
    expect(diff.map((t) => t.text).join('')).toBe('hello world');
  });

  it('produces an all-same diff for identical text', () => {
    const diff = computeWordDiff('make this shorter', 'make this shorter');
    expect(diff.every((t) => t.kind === 'same')).toBe(true);
    expect(diff.map((t) => t.text).join('')).toBe('make this shorter');
  });

  it('marks a full replacement as all-removed then all-added', () => {
    const diff = computeWordDiff('foo bar', 'baz qux');
    // Every actual word differs; a shared whitespace separator may still
    // legitimately match as `same` (it's the same literal token on both
    // sides), so only assert on the non-whitespace tokens here.
    const words = diff.filter((t) => t.text.trim().length > 0);
    expect(words.every((t) => t.kind !== 'same')).toBe(true);
    // Reconstructing each side (same + its own removed/added) must round-trip.
    expect(diff.filter((t) => t.kind !== 'added').map((t) => t.text).join('')).toBe('foo bar');
    expect(diff.filter((t) => t.kind !== 'removed').map((t) => t.text).join('')).toBe('baz qux');
  });

  it('interleaves same/removed/added tokens for a partial edit', () => {
    const diff = computeWordDiff('the quick brown fox', 'the slow brown fox jumps');
    // "the" and "brown fox" are shared; "quick" is removed, "slow" is added,
    // and a trailing "jumps" is added.
    expect(diff.some((t) => t.kind === 'same' && t.text === 'the')).toBe(true);
    expect(diff.some((t) => t.kind === 'removed' && t.text === 'quick')).toBe(true);
    expect(diff.some((t) => t.kind === 'added' && t.text === 'slow')).toBe(true);
    expect(diff.some((t) => t.kind === 'same' && t.text === 'brown')).toBe(true);
    expect(diff.some((t) => t.kind === 'same' && t.text === 'fox')).toBe(true);
    expect(diff.some((t) => t.kind === 'added' && t.text === 'jumps')).toBe(true);

    // Reconstructing original/proposed from same+removed / same+added must round-trip.
    const original = diff.filter((t) => t.kind !== 'added').map((t) => t.text).join('');
    const proposed = diff.filter((t) => t.kind !== 'removed').map((t) => t.text).join('');
    expect(original).toBe('the quick brown fox');
    expect(proposed).toBe('the slow brown fox jumps');
  });

  it('preserves whitespace tokens so reconstruction is exact', () => {
    const original = 'one  two\tthree';
    const proposed = 'one  two four';
    const diff = computeWordDiff(original, proposed);
    const reconstructedOriginal = diff.filter((t) => t.kind !== 'added').map((t) => t.text).join('');
    const reconstructedProposed = diff.filter((t) => t.kind !== 'removed').map((t) => t.text).join('');
    expect(reconstructedOriginal).toBe(original);
    expect(reconstructedProposed).toBe(proposed);
  });

  it('degrades to a single removed-run + single added-run beyond the token cap, and stays fast', () => {
    const original = Array.from({ length: 10_000 }, (_, i) => `orig${i}`).join(' ');
    const proposed = Array.from({ length: 10_000 }, (_, i) => `new${i}`).join(' ');

    const start = performance.now();
    const diff = computeWordDiff(original, proposed);
    const elapsedMs = performance.now() - start;

    // Degraded shape: exactly one removed token (the whole original) and one
    // added token (the whole proposed) — no per-word LCS attempted.
    expect(diff).toEqual([
      { kind: 'removed', text: original },
      { kind: 'added', text: proposed },
    ]);
    // A real O(n*m) LCS over 10k x 10k tokens would take seconds; the
    // degraded path must stay well under that.
    expect(elapsedMs).toBeLessThan(500);
  });

  it('still uses the real LCS diff comfortably under the cap', () => {
    // tokenize() keeps whitespace as its own tokens, so 999 words is
    // 999 + 998 separators = 1997 tokens/side — under the 2000 cap.
    const words = Array.from({ length: 999 }, (_, i) => `word${i}`);
    const original = words.join(' ');
    const proposed = words.join(' ');
    const diff = computeWordDiff(original, proposed);
    expect(diff.every((t) => t.kind === 'same')).toBe(true);
  });
});

describe('pickDiffLayout', () => {
  it('picks unified for short combined text', () => {
    expect(pickDiffLayout('make this shorter', 'shorter version')).toBe('unified');
  });

  it('picks side-by-side once combined length reaches the threshold', () => {
    const long = 'word '.repeat(50).trim();
    expect(pickDiffLayout(long, long)).toBe('side-by-side');
  });
});
