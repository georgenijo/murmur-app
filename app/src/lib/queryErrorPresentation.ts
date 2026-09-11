const QUERY_ERROR_MESSAGES: Record<string, string> = {
  not_configured: 'Choose a CLI executable in Voice Query settings.',
  invalid_executable: 'The configured CLI executable is missing or cannot be run.',
  invalid_arguments: 'The configured fixed arguments are invalid.',
  invalid_timeout: 'Choose a timeout between 5 seconds and 5 minutes.',
  invalid_environment: 'The saved Voice Query environment is invalid. Clear and re-enter it in Settings.',
  environment_unavailable: 'Murmur could not read the protected Voice Query environment. Open Settings and clear or re-save it.',
  busy: 'Murmur is already recording or running another local task.',
  audio_start_failed: 'The microphone could not start. Check the selected input and permission.',
  audio_capture_failed: 'Microphone capture failed while stopping. Check the selected input and try again.',
  audio_not_ready: 'The microphone was not ready yet. Try the shortcut again.',
  audio_recovering: 'Audio capture is recovering. Try again in a moment.',
  audio_recovery_stalled: 'Audio capture recovery stalled. Reopen Murmur and try again.',
  no_speech: 'No speech was detected. Try asking again.',
  empty_query: 'The recording did not contain a question.',
  query_too_large: 'The query and its included context exceeded the safety limit.',
  transcription_failed: 'Local transcription failed. Check the selected model.',
  spawn_failed: 'The configured CLI could not be started. Check its path and permissions.',
  timed_out: 'The configured CLI timed out and was stopped.',
  termination_unconfirmed: 'Murmur could not confirm that the CLI process stopped.',
  process_failed: 'The configured CLI process failed.',
  exit_nonzero: 'The configured CLI exited with an error.',
  provider_error: 'The configured provider reported an error.',
  unsupported_capabilities: 'This command does not match the trusted Claude profile. Open Settings and revoke workspace access or reselect Claude.',
  trusted_workspace_unavailable: 'The trusted folder changed or is unavailable. Open Settings, revoke access, and choose it again.',
  provider_not_authenticated: 'The configured provider is not signed in.',
  output_too_large: 'The answer exceeded the 256 KB safety limit and was stopped.',
  empty_answer: 'The configured CLI returned no answer.',
  clipboard_unavailable: 'The answer is ready, but the clipboard is unavailable. Use Copy to try again.',
  cancelled: 'The query was cancelled before it completed.',
};

const CODEX_INSTALL_INCOMPLETE = 'The Codex CLI installation is incomplete';

export function isIncompleteCodexDetail(errorDetail?: string | null): boolean {
  if (!errorDetail) return false;
  return errorDetail.includes(CODEX_INSTALL_INCOMPLETE)
    || (
      /ENOENT/i.test(errorDetail)
      && /@openai\/codex-darwin-/i.test(errorDetail)
      && /\/vendor\//i.test(errorDetail)
      && /\/codex\/codex/i.test(errorDetail)
    );
}

export function queryErrorMessage(errorCode: string | null, errorDetail?: string | null): string | null {
  // These are successful answers whose clipboard delivery was intentionally
  // skipped or deferred. They remain available through the explicit Copy action.
  if (!errorCode || [
    'audio_stalled',
    'clipboard_superseded',
    'auto_copy_disabled',
    'auto_copy_unavailable',
  ].includes(errorCode)) {
    return null;
  }
  if (
    errorCode === 'exit_nonzero'
    && isIncompleteCodexDetail(errorDetail)
  ) {
    return 'The Codex CLI installation is incomplete. Reinstall or update Codex, then try again.';
  }
  return QUERY_ERROR_MESSAGES[errorCode] ?? 'The voice query could not be completed.';
}

/**
 * Turns the content-free error code retained in query history into a useful
 * next step. Provider stderr remains ephemeral and never enters this path.
 */
export function queryHistoryErrorMessage(errorCode: string | null): string | null {
  if (errorCode && [
    'auto_copy_disabled',
    'auto_copy_unavailable',
    'clipboard_superseded',
    'clipboard_unavailable',
  ].includes(errorCode)) {
    return null;
  }
  const message = queryErrorMessage(errorCode);
  if (!message || !errorCode) return null;

  if ([
    'not_configured',
    'invalid_executable',
    'invalid_arguments',
    'invalid_timeout',
    'invalid_environment',
    'environment_unavailable',
    'spawn_failed',
    'process_failed',
    'exit_nonzero',
    'provider_error',
    'provider_not_authenticated',
    'empty_answer',
  ].includes(errorCode)) {
    return `${message} Open Settings > AI & Models > Voice Query and run Test before trying again.`;
  }
  if (errorCode === 'timed_out') {
    return `${message} Increase the timeout in Voice Query settings or ask a shorter question.`;
  }
  if (errorCode === 'query_too_large') {
    return `${message} Ask a shorter question, or close this popover and start a new query without the previous exchange.`;
  }
  if (errorCode === 'output_too_large') {
    return `${message} Ask for a shorter answer and try again.`;
  }
  if (errorCode === 'termination_unconfirmed') {
    return `${message} Quit and reopen Murmur before trying again.`;
  }
  if (errorCode === 'cancelled') {
    return `${message} Ask it again when you are ready.`;
  }
  if (errorCode === 'busy') {
    return `${message} Wait for that task to finish, then try again.`;
  }
  return message;
}
