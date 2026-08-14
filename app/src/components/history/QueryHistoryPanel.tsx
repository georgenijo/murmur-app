import { useState } from 'react';
import type { useQueryHistory } from '../../lib/hooks/useQueryHistory';

interface QueryHistoryPanelProps {
  history: ReturnType<typeof useQueryHistory>;
  retentionEnabled: boolean;
}

const PROVIDERS = [
  { value: 'all', label: 'All providers' },
  { value: 'claude', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
  { value: 'grok', label: 'Grok' },
  { value: 'cursor', label: 'Cursor' },
  { value: 'custom', label: 'Custom' },
] as const;

function providerLabel(provider: Exclude<(typeof PROVIDERS)[number]['value'], 'all'>): string {
  return PROVIDERS.find((option) => option.value === provider)?.label ?? provider;
}

function formatDuration(durationMs: number): string {
  if (durationMs < 1_000) return `${durationMs} ms`;
  if (durationMs < 10_000) return `${(durationMs / 1_000).toFixed(2)} s`;
  return `${(durationMs / 1_000).toFixed(1)} s`;
}

function tokenSummary(tokens: NonNullable<ReturnType<typeof useQueryHistory>['entries'][number]['tokens']>): string {
  const parts = [`${tokens.inputTokens.toLocaleString()} in`, `${tokens.outputTokens.toLocaleString()} out`];
  if (tokens.cachedInputTokens > 0) parts.push(`${tokens.cachedInputTokens.toLocaleString()} cached`);
  if (tokens.cacheCreationInputTokens > 0) parts.push(`${tokens.cacheCreationInputTokens.toLocaleString()} cache write`);
  if (tokens.reasoningOutputTokens > 0) parts.push(`${tokens.reasoningOutputTokens.toLocaleString()} reasoning`);
  return parts.join(' · ');
}

export function QueryHistoryPanel({ history, retentionEnabled }: QueryHistoryPanelProps) {
  const [notice, setNotice] = useState<string | null>(null);

  const purge = async () => {
    setNotice(null);
    if (await history.clear()) setNotice('Voice Query history deleted from this Mac.');
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="shrink-0 border-b border-outline-variant/20 px-4 py-3">
        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-0 flex-1">
            <h2 className="text-sm font-semibold text-on-surface">Voice Query history</h2>
            <p className="mt-0.5 text-[11px] leading-relaxed text-on-surface-variant">
              {retentionEnabled
                ? 'Questions and answers are kept only in Murmur’s bounded local store.'
                : 'Saving is off. Existing entries remain available until you delete them.'}
              {' '}Context is never stored as a separate field; provider stderr, commands, paths, and environment values are never stored here.
            </p>
          </div>
          <label className="text-[10px] font-medium uppercase tracking-wider text-on-surface-variant">
            Provider
            <select
              value={history.provider}
              onChange={(event) => history.setProvider(event.target.value as typeof history.provider)}
              className="mt-1 block rounded-lg border border-on-surface-variant bg-surface-container-lowest px-2 py-1.5 text-xs normal-case tracking-normal text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
            >
              {PROVIDERS.map((option) => (
                <option key={option.value} value={option.value}>{option.label}</option>
              ))}
            </select>
          </label>
          <button
            type="button"
            onClick={() => void history.refresh()}
            disabled={history.loading}
            className="rounded-lg border border-outline-variant/20 px-2.5 py-1.5 text-xs font-medium text-on-surface-variant hover:bg-surface-container disabled:opacity-50"
          >
            Refresh
          </button>
          <button
            type="button"
            onClick={() => void purge()}
            disabled={history.clearing}
            className="rounded-lg border border-error/20 px-2.5 py-1.5 text-xs font-medium text-error hover:bg-error/10 disabled:opacity-50"
          >
            {history.clearing ? 'Deleting…' : 'Delete all query history'}
          </button>
        </div>
        <div className="mt-2 flex items-center justify-between gap-3 text-[11px] text-on-surface-variant">
          <span>{history.total} of 200 local entries</span>
          {notice && <span role="status" className="text-success">{notice}</span>}
        </div>
        {history.error && (
          <p role="alert" className="mt-2 rounded-lg border border-error/20 bg-error/10 px-3 py-2 text-xs text-error">
            {history.error}
          </p>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {history.loading && history.entries.length === 0 ? (
          <div aria-label="Loading Voice Query history" className="space-y-3">
            {Array.from({ length: 4 }, (_, index) => (
              <div key={index} className="h-28 animate-pulse rounded-xl bg-surface-container" />
            ))}
          </div>
        ) : history.entries.length === 0 ? (
          <div className="grid min-h-full place-items-center text-center">
            <div className="max-w-sm rounded-2xl border border-dashed border-outline-variant/30 bg-surface-container-low p-8">
              <p className="text-sm font-medium text-on-surface">No saved Voice Queries</p>
              <p className="mt-1 text-xs leading-relaxed text-on-surface-variant">
                {retentionEnabled
                  ? 'Recognized Voice Queries appear here, including queries that shared app context. Saved answers can quote that context.'
                  : 'Turn on “Keep Voice Query history on this Mac” in Settings to save future questions and answers.'}
              </p>
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            {history.entries.map((entry) => (
              <article key={entry.id} className="rounded-xl border border-outline-variant/15 bg-surface-container-lowest p-4 shadow-sm">
                <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] text-on-surface-variant">
                  <span className="font-semibold text-on-surface">{providerLabel(entry.provider)}</span>
                  <span aria-hidden="true">·</span>
                  <time dateTime={new Date(entry.timestampMs).toISOString()}>
                    {new Date(entry.timestampMs).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' })}
                  </time>
                  <span aria-hidden="true">·</span>
                  <span>{formatDuration(entry.durationMs)}</span>
                  {entry.tokens && (
                    <>
                      <span aria-hidden="true">·</span>
                      <span>{tokenSummary(entry.tokens)}</span>
                    </>
                  )}
                  {entry.errorCode && (
                    <span className="ml-auto rounded-full bg-error/10 px-2 py-0.5 font-mono text-error">
                      {entry.errorCode}
                    </span>
                  )}
                </div>
                <div className="mt-3 grid gap-3 md:grid-cols-2">
                  <section aria-label="Question" className="min-w-0 rounded-lg bg-surface-container-low p-3">
                    <h3 className="text-[10px] font-semibold uppercase tracking-wider text-on-surface-variant">Question</h3>
                    <p className="mt-1 whitespace-pre-wrap break-words text-sm text-on-surface">{entry.question}</p>
                  </section>
                  <section aria-label="Answer" className="min-w-0 rounded-lg bg-surface-container-low p-3">
                    <h3 className="text-[10px] font-semibold uppercase tracking-wider text-on-surface-variant">Answer</h3>
                    <p className="mt-1 whitespace-pre-wrap break-words text-sm text-on-surface">
                      {entry.answer || (entry.errorCode ? 'No answer was returned.' : 'Empty answer')}
                    </p>
                  </section>
                </div>
              </article>
            ))}
            {history.hasMore && (
              <div className="flex justify-center pt-1">
                <button
                  type="button"
                  onClick={() => void history.loadMore()}
                  disabled={history.loading}
                  className="rounded-lg border border-outline-variant/20 px-4 py-2 text-xs font-semibold text-primary hover:bg-surface-container disabled:opacity-50"
                >
                  {history.loading ? 'Loading…' : 'Load more'}
                </button>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
