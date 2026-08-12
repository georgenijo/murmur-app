import { describe, expect, it } from 'vitest';

import { queryErrorMessage } from './QueryReviewApp';

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
