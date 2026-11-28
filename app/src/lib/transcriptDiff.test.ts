import { describe, expect, it } from 'vitest';
import { changedStageLabels, countDiffChanges, diffTranscript, MAX_DIFF_CELLS } from './transcriptDiff';

const join = (segments: { kind: string; text: string }[], keep: 'raw' | 'delivered') =>
  segments
    .filter((segment) => segment.kind === 'same' || segment.kind === (keep === 'raw' ? 'removed' : 'added'))
    .map((segment) => segment.text)
    .join('');

describe('diffTranscript', () => {
  it('reports identical text as one unchanged run', () => {
    expect(diffTranscript('ship it today', 'ship it today')).toEqual([{ kind: 'same', text: 'ship it today' }]);
  });

  it('shows removed filler and a capitalization replacement', () => {
    const segments = diffTranscript('um so we ship on friday', 'So we ship on Friday.')!;
    expect(segments).toEqual([
      { kind: 'removed', text: 'um so' },
      { kind: 'added', text: 'So' },
      { kind: 'same', text: ' we ship on ' },
      { kind: 'removed', text: 'friday' },
      { kind: 'added', text: 'Friday.' },
    ]);
    expect(countDiffChanges(segments)).toBe(2);
  });

  it('reassembles the delivered text exactly from same+added runs', () => {
    const raw = 'open the tori config new line then run npm test';
    const delivered = 'Open the Tauri config\nthen run npm test';
    const segments = diffTranscript(raw, delivered)!;
    expect(join(segments, 'delivered')).toBe(delivered);
  });

  it('keeps a spoken line break visible as an addition', () => {
    const segments = diffTranscript('first new line second', 'first\nsecond')!;
    expect(segments.some((segment) => segment.kind === 'added' && segment.text.includes('\n'))).toBe(true);
    expect(segments.some((segment) => segment.kind === 'removed' && segment.text.includes('new line'))).toBe(true);
  });

  it('folds a spacing-only change into unchanged text', () => {
    const segments = diffTranscript('a  b', 'a b')!;
    expect(segments.every((segment) => segment.kind === 'same')).toBe(true);
    expect(countDiffChanges(segments)).toBe(0);
  });

  it('handles empty sides', () => {
    expect(diffTranscript('', 'Hello')).toEqual([{ kind: 'added', text: 'Hello' }]);
    expect(diffTranscript('hello', '')).toEqual([{ kind: 'removed', text: 'hello' }]);
    expect(diffTranscript('', '')).toEqual([]);
  });

  it('refuses a pair beyond the comparison bound instead of blocking', () => {
    const words = Math.ceil(Math.sqrt(MAX_DIFF_CELLS));
    const long = Array.from({ length: words }, (_, index) => `w${index}`).join(' ');
    expect(diffTranscript(long, `${long} more`)).toBeNull();
  });
});

describe('changedStageLabels', () => {
  it('names only stages that changed text, deduplicated, in order', () => {
    expect(changedStageLabels([
      { stage: 'cleanup', outcome: 'applied', changed: true },
      { stage: 'smart_correction', outcome: 'applied', changed: false },
      { stage: 'spoken_numbers', outcome: 'applied', changed: true },
      { stage: 'cleanup', outcome: 'applied', changed: true },
      { stage: 'future_stage', outcome: 'applied', changed: true },
    ])).toEqual(['Cleanup', 'Spoken numbers', 'Future stage']);
  });

  it('returns nothing for missing stage data', () => {
    expect(changedStageLabels(undefined)).toEqual([]);
  });
});
