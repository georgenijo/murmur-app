import { useEffect, useMemo } from 'react';
import Markdown from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import { useQueryReviewDriver, type QueryContextDisplay, type QueryReviewState } from '../../lib/hooks/useQueryReviewDriver';
import { queryErrorFix, queryErrorMessage } from '../../lib/queryErrors';

export { queryErrorMessage };

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
    case 'failed':
      return errorCode === 'provider_not_authenticated' ? 'The CLI is not signed in' : 'Voice query failed';
    default: return 'Voice Query';
  }
}

function formatSelectionBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / 1024).toFixed(1)} KB`;
}

export function queryContextSummary(context: QueryContextDisplay | null): string | null {
  if (!context) return null;
  if (context.status === 'excluded') return 'Context: excluded for this app';
  if (context.status === 'unavailable') return 'Context: unavailable';
  const parts = [context.appName || 'Unknown app'];
  if (context.windowTitle) parts.push(context.windowTitle);
  if (context.selectionBytes !== null) {
    parts.push(`${formatSelectionBytes(context.selectionBytes)} selection${context.selectionTruncated ? ' (trimmed)' : ''}`);
  }
  return `Context: ${parts.join(' — ')}`;
}

export function QueryReviewApp() {
  const driver = useQueryReviewDriver();
  const errorMessage = useMemo(() => queryErrorMessage(driver.errorCode), [driver.errorCode]);
  const contextSummary = useMemo(() => queryContextSummary(driver.context), [driver.context]);
  const fix = useMemo(
    () => queryErrorFix(driver.errorCode, driver.signIn?.hint),
    [driver.errorCode, driver.signIn],
  );
  const terminal = driver.state === 'ready' || driver.state === 'failed';
  const failed = driver.state === 'failed';
  // On a failure the CLI's own stdout is evidence, not the answer. It used to
  // win over the error message (`answer || errorMessage`), so a provider that
  // printed "Not logged in" and exited non-zero looked like it had answered.
  const showsAnswerBody = Boolean(driver.answer) && !failed;
  const offersSignIn = failed && driver.errorCode === 'provider_not_authenticated' && driver.signIn !== null;

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
          {contextSummary && (
            <p title={contextSummary} className="mt-0.5 truncate text-[11px] text-white/45">{contextSummary}</p>
          )}
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
            {showsAnswerBody
              ? <Markdown rehypePlugins={[rehypeSanitize]}>{driver.answer}</Markdown>
              : (
                <div className="space-y-2">
                  <p className="whitespace-pre-wrap font-medium text-white/90">
                    {errorMessage || 'No answer was returned.'}
                  </p>
                  {fix && <p className="whitespace-pre-wrap text-white/70">{fix}</p>}
                  {driver.errorDetail && (
                    <div>
                      <p className="text-[10px] font-semibold uppercase tracking-[0.12em] text-white/40">
                        {driver.signIn ? `${driver.signIn.provider} said` : 'The CLI said'}
                      </p>
                      <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-black/40 p-2 text-[11px] leading-snug text-white/70">
                        {driver.errorDetail}
                      </pre>
                    </div>
                  )}
                  {failed && driver.answer && (
                    <div>
                      <p className="text-[10px] font-semibold uppercase tracking-[0.12em] text-white/40">Partial output</p>
                      <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-black/40 p-2 text-[11px] leading-snug text-white/70">
                        {driver.answer}
                      </pre>
                    </div>
                  )}
                </div>
              )}
            {driver.state === 'running' && <span aria-hidden="true" className="ml-0.5 inline-block h-3 w-px animate-pulse bg-white/60 align-middle" />}
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
              {offersSignIn && (
                <button type="button" onClick={driver.startSignIn} className="rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white hover:bg-white/15">
                  Sign in…
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
