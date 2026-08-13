import { describe, expect, it } from 'vitest';

import { queryErrorFix, queryErrorMessage } from './queryErrors';

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
    expect(queryErrorMessage('provider_not_authenticated'))
      .toBe('The CLI is not signed in, so it refused the question.');
  });

  it('falls back to a generic message for an unrecognised code', () => {
    expect(queryErrorMessage('something_new')).toBe('The voice query could not be completed.');
  });
});

describe('queryErrorFix', () => {
  it('names the exact command for a signed-out provider', () => {
    expect(queryErrorFix('provider_not_authenticated', 'claude auth login'))
      .toBe('Run claude auth login in Terminal, or use Sign in… below.');
  });

  it('invents no advice without a known login or a matching failure', () => {
    expect(queryErrorFix('provider_not_authenticated', null)).toBeNull();
    expect(queryErrorFix('timed_out', 'claude auth login')).toBeNull();
    expect(queryErrorFix(null, 'claude auth login')).toBeNull();
  });
});
