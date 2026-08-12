import { useEffect, useMemo } from 'react';
import Markdown from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import { useQueryReviewDriver, type QueryReviewState } from '../../lib/hooks/useQueryReviewDriver';

const ERROR_MESSAGES: Record<string, string> = {
  not_configured: 'Choose a CLI executable in Voice Query settings.',
  invalid_executable: 'The configured CLI executable is missing or cannot be run.',
  invalid_arguments: 'The configured fixed arguments are invalid.',
  invalid_timeout: 'Choose a timeout between 5 seconds and 5 minutes.',
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
  provider_error: 'The configured provider reported an error.',
  provider_not_authenticated: 'The configured provider is not signed in.',
  output_too_large: 'The answer exceeded the 256 KB safety limit and was stopped.',
  empty_answer: 'The configured CLI returned no answer.',
  clipboard_unavailable: 'The answer is ready, but the clipboard is unavailable. Use Copy to try again.',
};

function statusLabel(state: QueryReviewState, errorCode: string | null): string {
  switch (state) {
    case 'connecting': return 'Connecting microphone…';
    case 'listening': return 'Listening — tap the query key once when done';
    case 'transcribing': return 'Transcribing locally…';
    case 'running': return 'Agent is answering…';
    case 'ready':
      if (errorCode === 'clipboard_unavailable') return 'Answer ready';
      if (errorCode === 'clipboard_superseded') return 'Answer ready — clipboard left alone';
      return 'Answer copied to clipboard';
    case 'failed': return 'Voice query failed';
    default: return 'Voice Query';
  }
}

export function queryErrorMessage(errorCode: string | null): string | null {
  // `clipboard_superseded` is a successful answer whose auto-copy deferred to a
  // clipboard write the user made while it was generating — not a failure.
  if (!errorCode || errorCode === 'audio_stalled' || errorCode === 'clipboard_superseded') {
    return null;
  }
  return ERROR_MESSAGES[errorCode] ?? 'The voice query could not be completed.';
}

export function QueryReviewApp() {
  const driver = useQueryReviewDriver();
  const errorMessage = useMemo(() => queryErrorMessage(driver.errorCode), [driver.errorCode]);
  const terminal = driver.state === 'ready' || driver.state === 'failed';
  const primaryText = driver.state === 'failed'
    ? errorMessage ?? 'The voice query could not be completed.'
    : driver.answer || errorMessage || (terminal ? 'No answer was returned.' : '');

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        driver.cancel();
      } else if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'c' && driver.state === 'ready') {
        event.preventDefault();
        driver.copy();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [driver.cancel, driver.copy, driver.state]);

  return (
    <main
      className="query-review-surface flex h-full w-full select-none flex-col overflow-hidden rounded-[16px] border border-white/10 bg-[#141414]/95 text-white shadow-2xl backdrop-blur-3xl"
      aria-label="Voice Query"
    >
      <header className="flex min-h-[64px] items-center gap-3 px-4 py-3">
        <span
          aria-hidden="true"
          className={`h-2.5 w-2.5 shrink-0 rounded-full ${driver.state === 'failed' ? 'bg-red-400' : driver.state === 'ready' ? 'bg-emerald-400' : 'animate-pulse bg-violet-400'}`}
        />
        <div className="min-w-0 flex-1">
          <p className="text-[11px] font-semibold uppercase tracking-[0.14em] text-white/45">Voice Query</p>
          <p aria-live="polite" className="mt-0.5 truncate text-[13px] font-medium text-white/90">
            {statusLabel(driver.state, driver.errorCode)}
          </p>
        </div>
        {!terminal && (
          <button type="button" onClick={driver.cancel} className="rounded-lg px-2.5 py-1.5 text-xs font-medium text-white/60 hover:bg-white/10 hover:text-white">
            Cancel
          </button>
        )}
      </header>

      {(driver.answer || errorMessage || terminal) && (
        <section className="flex min-h-0 flex-1 flex-col border-t border-white/10">
          <div
            aria-label="Query answer"
            aria-live="polite"
            className="min-h-0 flex-1 select-text overflow-y-auto break-words px-4 py-3 text-[13px] leading-relaxed text-white/85 [&>*:first-child]:mt-0 [&>*:last-child]:mb-0 [&_a]:text-violet-300 [&_a]:underline [&_blockquote]:my-2 [&_blockquote]:border-l-2 [&_blockquote]:border-white/20 [&_blockquote]:pl-3 [&_blockquote]:text-white/70 [&_code]:rounded [&_code]:bg-white/10 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-[12px] [&_em]:italic [&_h1]:mb-1 [&_h1]:mt-3 [&_h1]:text-[15px] [&_h1]:font-semibold [&_h1]:text-white [&_h2]:mb-1 [&_h2]:mt-3 [&_h2]:text-[14px] [&_h2]:font-semibold [&_h2]:text-white [&_h3]:mb-1 [&_h3]:mt-2 [&_h3]:text-[13px] [&_h3]:font-semibold [&_h3]:text-white [&_hr]:my-3 [&_hr]:border-white/10 [&_li]:my-0.5 [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_p]:my-2 [&_pre]:my-2 [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:bg-black/40 [&_pre]:p-3 [&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_strong]:font-semibold [&_strong]:text-white [&_table]:my-2 [&_table]:w-full [&_td]:pr-3 [&_th]:pr-3 [&_th]:text-left [&_th]:font-semibold [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5"
          >
            {driver.state === 'failed'
              ? <p className="whitespace-pre-wrap">{primaryText}</p>
              : driver.answer
                ? <Markdown rehypePlugins={[rehypeSanitize]}>{driver.answer}</Markdown>
                : <p className="whitespace-pre-wrap">{primaryText}</p>}
            {driver.state === 'running' && <span aria-hidden="true" className="ml-0.5 inline-block h-3 w-px animate-pulse bg-white/60 align-middle" />}
            {driver.state === 'failed' && driver.errorDetail && (
              <div className="mt-3 rounded-lg border border-red-300/15 bg-red-950/30 p-2.5">
                <p className="select-none text-[10px] font-semibold uppercase tracking-[0.12em] text-red-200/55">
                  Provider detail
                </p>
                <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-red-100/75">
                  {driver.errorDetail}
                </pre>
              </div>
            )}
            {driver.errorCode === 'provider_not_authenticated' && driver.signInFix && (
              <p className="mt-3 text-xs text-amber-100/80">{driver.signInFix}</p>
            )}
            {driver.signInStatus && (
              <p aria-live="polite" className="mt-2 text-xs text-white/60">{driver.signInStatus}</p>
            )}
          </div>
          <footer className="flex items-center justify-between border-t border-white/10 px-3 py-2">
            <span className={`text-[10px] ${driver.errorCode === 'clipboard_unavailable' ? 'text-amber-300/80' : 'text-white/35'}`}>
              {driver.errorCode === 'clipboard_unavailable'
                ? 'Clipboard unavailable · never auto-pasted'
                : driver.errorCode === 'clipboard_superseded'
                  ? 'Clipboard left as-is · press Copy for the answer'
                  : driver.state === 'ready' ? 'Never auto-pasted' : 'Esc to cancel'}
            </span>
            <div className="flex gap-2">
              {driver.errorCode === 'provider_not_authenticated' && driver.signInFix && (
                <button
                  type="button"
                  disabled={driver.signInBusy}
                  onClick={() => void driver.signIn()}
                  className="rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white hover:bg-white/15 disabled:cursor-wait disabled:opacity-50"
                >
                  {driver.signInBusy ? 'Waiting…' : 'Sign in…'}
                </button>
              )}
              {driver.state === 'ready' && (
                <button type="button" onClick={driver.copy} className="rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white hover:bg-white/15">
                  Copy
                </button>
              )}
              {terminal && (
                <button type="button" onClick={driver.cancel} className="rounded-lg bg-violet-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-violet-400">
                  Close
                </button>
              )}
            </div>
          </footer>
        </section>
      )}
    </main>
  );
}
