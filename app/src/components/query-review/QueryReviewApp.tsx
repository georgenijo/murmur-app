import { useEffect, useMemo, useRef } from 'react';
import { ArrowUpRight, Copy, X } from 'lucide-react';
import Markdown from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import { DashboardAction } from '../ui/DashboardPrimitives';
import { useQueryReviewDriver } from '../../lib/hooks/useQueryReviewDriver';
import { isIncompleteCodexDetail, queryErrorMessage } from '../../lib/queryErrorPresentation';
import { formatQueryCost, type QueryUsage } from '../../lib/queryUsage';
import type { QueryReviewState } from '../../lib/queryReview';
import './query-review.css';

export { queryErrorMessage };

export function statusLabel(state: QueryReviewState, _errorCode: string | null): string {
  switch (state) {
    case 'connecting': return 'Connecting microphone…';
    case 'listening': return 'Listening…';
    case 'transcribing': return 'Transcribing…';
    case 'running': return 'Thinking…';
    case 'ready': return 'Answer ready';
    case 'failed': return 'Couldn’t finish';
    default: return 'Voice Query';
  }
}

export function queryListeningPartial(state: QueryReviewState, partial: string): string | null {
  return state === 'listening' && partial.trim() ? partial : null;
}

export function formatQueryUsage(usage: QueryUsage | null): string | null {
  if (!usage) return null;
  const parts = [`${usage.inputTokens.toLocaleString()} in`, `${usage.outputTokens.toLocaleString()} out`];
  if (usage.costUsd !== null) parts.push(formatQueryCost(usage.costUsd));
  return parts.join(' · ');
}

export function QueryReviewApp() {
  const driver = useQueryReviewDriver();
  return <QueryReviewView driver={driver} />;
}

export function QueryReviewView({ driver }: { driver: ReturnType<typeof useQueryReviewDriver> }) {
  const detailsRef = useRef<HTMLDetailsElement>(null);
  const errorMessage = useMemo(
    () => queryErrorMessage(driver.errorCode, driver.errorDetail),
    [driver.errorCode, driver.errorDetail],
  );
  const listeningPartial = queryListeningPartial(driver.state, driver.partial);
  const terminal = driver.state === 'ready' || driver.state === 'failed';
  const showErrorDetail = driver.state === 'failed' && driver.errorDetail && !isIncompleteCodexDetail(driver.errorDetail);
  const usageText = driver.state === 'ready' ? formatQueryUsage(driver.usage) : null;
  const primaryText = driver.state === 'failed'
    ? errorMessage ?? 'The voice query could not be completed.'
    : driver.answer || errorMessage || (terminal ? 'No answer was returned.' : '');

  useEffect(() => {
    if (detailsRef.current) detailsRef.current.open = false;
  }, [driver.state]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        driver.cancel();
      } else if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'c' && driver.state === 'ready') {
        if (window.getSelection()?.toString()) return;
        event.preventDefault();
        driver.copy();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [driver.cancel, driver.copy, driver.state]);

  return (
    <main className="query-review-surface" aria-label="Voice Query" data-state={driver.state}>
      <header className="query-review-header">
        <div className="query-review-heading">
          <span className="query-review-dot" aria-hidden="true" />
          <span className="query-review-title">Voice Query</span>
          <span className="query-review-status" role="status">{statusLabel(driver.state, driver.errorCode)}</span>
        </div>
        <button type="button" className="ui-icon-button" onClick={driver.cancel} aria-label={terminal ? 'Close' : 'Cancel'} title={terminal ? 'Close (Esc)' : 'Cancel (Esc)'}>
          <X aria-hidden="true" />
        </button>
      </header>
      <div className="query-review-content">
        {listeningPartial ? (
          <p aria-label="Heard so far" aria-live="polite" className="query-review-plain">{listeningPartial}</p>
        ) : <>
          {driver.question && <section aria-label="Your question" className="query-review-message">
            <h2>You</h2>
            <p className="query-review-plain">{driver.question}</p>
          </section>}
          {(driver.answer || errorMessage || terminal) && <section aria-label="Query answer" className="query-review-message" data-speaker="assistant">
            <h2>Assistant</h2>
            <div className="query-review-markdown" aria-live="polite">
              {driver.state !== 'failed' && driver.answer
                ? <Markdown rehypePlugins={[rehypeSanitize]} components={{ img: () => null }}>{driver.answer}</Markdown>
                : <p className="query-review-plain">{primaryText}</p>}
            </div>
            {showErrorDetail && <details className="query-review-provider-detail">
              <summary>Provider detail</summary><pre>{driver.errorDetail}</pre>
            </details>}
            {driver.errorCode === 'provider_not_authenticated' && driver.signInFix && <p className="query-review-notice">{driver.signInFix}</p>}
            {driver.signInStatus && <p role="status" className="query-review-notice">{driver.signInStatus}</p>}
          </section>}
        </>}
        {driver.state === 'listening' && <p className="query-review-hint">Tap the query key to finish.</p>}
        {driver.followUpError && <p role="status" className="query-review-notice">{driver.followUpError}</p>}
        {driver.assistantOpenError && <p role="status" className="query-review-notice">{driver.assistantOpenError} Open Assistant in Murmur to check the connection.</p>}
        {driver.state === 'ready' && driver.errorCode === 'clipboard_unavailable' && <p role="status" className="query-review-notice">Couldn’t copy. Try Copy again.</p>}
        <details ref={detailsRef} className="query-review-details">
          <summary>{driver.contextSummary ? 'Details · context included' : 'Details'}</summary>
          {driver.capabilitySummary && <p aria-label="Query capabilities">{driver.capabilitySummary}</p>}
          {driver.contextSummary && <p aria-label="Query context">{driver.contextSummary}</p>}
          {usageText && <p>{usageText}</p>}
          {driver.canOpenInAssistant && <p>Open in Assistant keeps this exchange in Murmur. Follow-ups use Pi’s private sessions.</p>}
        </details>
      </div>
      {terminal && <footer className="query-review-footer">
        <div className="query-review-actions">
          {driver.canOpenInAssistant && <button type="button" className="ui-dashboard-action" data-action="primary" disabled={driver.assistantBusy || driver.followUpBusy} onClick={() => void driver.openInAssistant()} title="Keep this exchange and continue in Assistant">
            {driver.assistantBusy ? 'Opening…' : 'Open in Assistant'}<ArrowUpRight aria-hidden="true" />
          </button>}
          {driver.errorCode === 'provider_not_authenticated' && driver.signInFix && <DashboardAction variant="secondary" disabled={driver.signInBusy} onActivate={() => void driver.signIn()}>{driver.signInBusy ? 'Waiting…' : 'Sign in…'}</DashboardAction>}
          {driver.state === 'ready' && <DashboardAction variant="quiet" disabled={driver.followUpBusy || driver.assistantBusy} onActivate={() => void driver.followUp()}>{driver.followUpBusy ? 'Starting…' : 'Ask follow-up'}</DashboardAction>}
        </div>
        {driver.state === 'ready' && <button type="button" className="ui-icon-button" onClick={driver.copy} aria-label="Copy answer" title="Copy answer"><Copy aria-hidden="true" /></button>}
      </footer>}
    </main>
  );
}
