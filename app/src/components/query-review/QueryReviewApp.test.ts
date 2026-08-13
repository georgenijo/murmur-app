import { describe, expect, it } from 'vitest';

import { queryContextSummary, queryErrorMessage } from './QueryReviewApp';

describe('queryErrorMessage', () => {
  it('treats a deferred clipboard as success, not failure', () => {
    // The answer arrived fine; its auto-copy simply stood aside for a clipboard
    // write the user made while it was generating. Surfacing this as an error
    // would read as though the query itself failed.
    expect(queryErrorMessage('clipboard_superseded')).toBeNull();
  });

  it('stays silent for non-terminal audio stalls and no error at all', () => {
    expect(queryErrorMessage('audio_stalled')).toBeNull();
    expect(queryErrorMessage(null)).toBeNull();
  });

  it('explains real failures', () => {
    expect(queryErrorMessage('timed_out')).toBe('The configured CLI timed out and was stopped.');
    expect(queryErrorMessage('empty_answer')).toBe('The configured CLI returned no answer.');
  });

  it('falls back to a generic message for an unrecognised code', () => {
    expect(queryErrorMessage('something_new')).toBe('The voice query could not be completed.');
  });
});

describe('queryContextSummary', () => {
  it('shows exactly which context was attached without exposing selected text', () => {
    expect(queryContextSummary({
      status: 'included',
      appName: 'Safari',
      windowTitle: 'Murmur issue',
      selectionBytes: 1229,
      selectionTruncated: false,
    })).toBe('Context: Safari — Murmur issue — 1.2 KB selection');
  });

  it('makes exclusions and unavailable context visible', () => {
    expect(queryContextSummary({ status: 'excluded', appName: null, windowTitle: null, selectionBytes: null, selectionTruncated: false }))
      .toBe('Context: excluded for this app');
    expect(queryContextSummary({ status: 'unavailable', appName: null, windowTitle: null, selectionBytes: null, selectionTruncated: false }))
      .toBe('Context: unavailable');
  });
});
