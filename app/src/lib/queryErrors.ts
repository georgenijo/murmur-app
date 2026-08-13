/**
 * Stable voice-query error codes and the sentences shown for them.
 *
 * Shared by the answer popover and Settings so a preflight refusal and a
 * mid-question failure read identically — the same code always produces the
 * same explanation and, where one exists, the same fix.
 */

export const QUERY_ERROR_MESSAGES: Record<string, string> = {
  not_configured: 'Choose a CLI executable in Voice Query settings.',
  invalid_executable: 'The configured CLI executable is missing or cannot be run.',
  invalid_arguments: 'The configured fixed arguments are invalid.',
  invalid_timeout: 'Choose a timeout between 5 seconds and 5 minutes.',
  invalid_preset: 'That provider preset is not recognised. Choose one again in Voice Query settings.',
  busy: 'Murmur is already recording or running another local task.',
  audio_start_failed: 'The microphone could not start. Check the selected input and permission.',
  audio_not_ready: 'The microphone was not ready yet. Try the shortcut again.',
  audio_recovering: 'Audio capture is recovering. Try again in a moment.',
  audio_recovery_stalled: 'Audio capture recovery stalled. Reopen Murmur and try again.',
  no_speech: 'No speech was detected. Try asking again.',
  empty_query: 'The recording did not contain a question.',
  query_too_large: 'The spoken query exceeded the safety limit.',
  transcription_failed: 'Local transcription failed. Check the selected model.',
  spawn_failed: 'The configured CLI could not be started. Check its path and permissions.',
  timed_out: 'The configured CLI timed out and was stopped.',
  termination_unconfirmed: 'Murmur could not confirm that the CLI process stopped.',
  process_failed: 'The configured CLI process failed.',
  exit_nonzero: 'The configured CLI exited with an error.',
  provider_not_authenticated: 'The CLI is not signed in, so it refused the question.',
  output_too_large: 'The answer exceeded the 256 KB safety limit and was stopped.',
  empty_answer: 'The configured CLI returned no answer.',
  clipboard_unavailable: 'The answer is ready, but the clipboard is unavailable. Use Copy to try again.',
};

/**
 * Codes that describe a state rather than a failure. `clipboard_superseded` is
 * a successful answer whose auto-copy deferred to a clipboard write the user
 * made while it was generating; `audio_stalled` is a slow start, not an end.
 */
const NON_FAILURE_CODES = new Set(['audio_stalled', 'clipboard_superseded']);

export function queryErrorMessage(errorCode: string | null): string | null {
  if (!errorCode || NON_FAILURE_CODES.has(errorCode)) return null;
  return QUERY_ERROR_MESSAGES[errorCode] ?? 'The voice query could not be completed.';
}

/**
 * The exact terminal command that fixes a not-signed-in provider, e.g.
 * `Run claude auth login in Terminal.` Only auth failures have a known fix, so
 * everything else returns null rather than inventing advice.
 */
export function queryErrorFix(errorCode: string | null, loginHint: string | null | undefined): string | null {
  if (errorCode !== 'provider_not_authenticated' || !loginHint) return null;
  return `Run ${loginHint} in Terminal, or use Sign in… below.`;
}
