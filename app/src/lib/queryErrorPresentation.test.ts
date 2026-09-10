import { describe, expect, it } from 'vitest';
import { queryHistoryErrorMessage } from './queryErrorPresentation';

describe('queryHistoryErrorMessage', () => {
  it('adds a provider recovery step without requiring saved stderr', () => {
    expect(queryHistoryErrorMessage('exit_nonzero')).toBe(
      'The configured CLI exited with an error. Open Settings > AI & Models > Voice Query and run Test before trying again.',
    );
  });

  it('keeps task-specific recovery guidance', () => {
    expect(queryHistoryErrorMessage('timed_out')).toBe(
      'The configured CLI timed out and was stopped. Increase the timeout in Voice Query settings or ask a shorter question.',
    );
    expect(queryHistoryErrorMessage('no_speech')).toBe('No speech was detected. Try asking again.');
  });

  it('does not turn successful delivery states into failures', () => {
    expect(queryHistoryErrorMessage('auto_copy_disabled')).toBeNull();
    expect(queryHistoryErrorMessage('clipboard_superseded')).toBeNull();
    expect(queryHistoryErrorMessage('clipboard_unavailable')).toBeNull();
  });
});
