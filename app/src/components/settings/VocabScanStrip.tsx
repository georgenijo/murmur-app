import { useState } from 'react';
import type { VocabScanSummary } from '../../lib/settings';
import type {
  VocabScanStatus,
  VocabScanStats,
  WalkerRow,
} from '../../lib/hooks/useVocabScan';
import { VocabTermsModal } from './VocabTermsModal';

interface VocabScanStripProps {
  status: VocabScanStatus;
  walker: WalkerRow[];
  stats: VocabScanStats;
  /** Absolute path of the folder being / to be scanned. Empty = none chosen. */
  folder: string;
  onScan: () => void;
  onCancel: () => void;
}

/** Bytes -> compact "2.3 MB" / "812 KB" string. */
function formatBytes(bytes: number): string {
  if (bytes <= 0) return '0 MB';
  const mb = bytes / (1024 * 1024);
  if (mb >= 1) return `${mb.toFixed(1)} MB`;
  const kb = bytes / 1024;
  return `${Math.round(kb)} KB`;
}

function formatMs(ms: number): string {
  if (ms <= 0) return '0.0s';
  return `${(ms / 1000).toFixed(1)}s`;
}

// The backend streams file reads and skipped dirs only (no dir-descent callback),
// so the walker has just two row kinds.
const ROW_ICON: Record<WalkerRow['kind'], string> = {
  file: '›',
  skip: '⊘',
};

const ROW_CLASS: Record<WalkerRow['kind'], string> = {
  file: 'text-stone-500 dark:text-stone-500',
  skip: 'text-stone-400 dark:text-stone-600 line-through opacity-60',
};

/**
 * Three-state scan feedback strip (idle / scanning / done|empty) shown under the
 * Project Folder field. Idle copy stays honest about the built-in dev terms;
 * scanning streams the live walker + ticking counts; done surfaces the result
 * stats, the amber 1000-file cap warning, and sample-term chips.
 */
export function VocabScanStrip({
  status,
  walker,
  stats,
  folder,
  onScan,
  onCancel,
}: VocabScanStripProps) {
  const [modalOpen, setModalOpen] = useState(false);
  const scanning = status === 'scanning';
  const summary: VocabScanSummary | null = stats.summary;
  // Result card needs the full Summary; only render it once invoke() has resolved.
  const showResult = (status === 'done' || status === 'empty') && summary !== null;
  // The walk has settled (terminal status) even if the Summary hasn't landed yet.
  // Keep the progress bar full across that gap so it doesn't collapse to 0% for a
  // few ms between the done tick and the invoke() resolution.
  const settled = status === 'done' || status === 'empty';

  // Status dot color.
  const dotClass =
    status === 'scanning'
      ? 'bg-amber-500 animate-pulse'
      : status === 'done'
        ? 'bg-green-500'
        : status === 'empty'
          ? 'bg-amber-500'
          : 'bg-stone-400 dark:bg-stone-500';

  // Header label + sub copy per state. The terminal 'done'/'empty' status lands a
  // few ms before invoke() resolves (the backend's final progress tick sets it),
  // so `summary` can be briefly null while status is already terminal. Fall back
  // to the live stats counters for the sub copy in that window so the header never
  // flickers to the idle "Not scanned yet" text between the done tick and resolve.
  let label: string;
  let sub: string;
  if (scanning) {
    label = 'Scanning…';
    sub = `${stats.filesRead} files · ${stats.dirsSkipped} dirs skipped · ${stats.termsSoFar} terms`;
  } else if (status === 'done') {
    label = 'Scan complete';
    sub = summary
      ? `${summary.terms} terms layered on top of built-ins · just now`
      : `${stats.termsSoFar} terms layered on top of built-ins · just now`;
  } else if (status === 'empty') {
    label = 'No identifiers found';
    sub = 'Folder had no source files — built-in dev terms still active.';
  } else {
    label = 'Not scanned yet';
    sub = 'Built-in dev terms active. Scan this folder to add your own identifiers.';
  }

  // Primary action button.
  const actionLabel = scanning
    ? 'Cancel'
    : status === 'done' || status === 'empty'
      ? 'Rescan'
      : 'Scan now';
  const actionDisabled = !scanning && !folder;

  return (
    <div className="mt-3.5 overflow-hidden rounded-xl border border-stone-200 bg-stone-50 dark:border-stone-800 dark:bg-stone-950">
      {/* status row */}
      <div className="flex items-center gap-2.5 px-3 py-3">
        <span className={`h-2 w-2 shrink-0 rounded-full ${dotClass}`} />
        <div className="min-w-0 flex-1">
          <div className="text-xs font-medium text-stone-700 dark:text-stone-300">{label}</div>
          <div className="mt-0.5 truncate text-[11px] tabular-nums text-stone-500 dark:text-stone-500">
            {sub}
          </div>
        </div>
        <button
          type="button"
          onClick={scanning ? onCancel : onScan}
          disabled={actionDisabled}
          className={`shrink-0 whitespace-nowrap rounded-md border px-3 py-1.5 text-[11px] font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
            scanning
              ? 'border-stone-300 text-stone-500 hover:border-stone-400 dark:border-stone-700 dark:text-stone-400'
              : 'border-stone-300 bg-white text-stone-700 hover:border-stone-400 dark:border-stone-600 dark:bg-stone-800 dark:text-stone-200 dark:hover:border-stone-400'
          }`}
        >
          {actionLabel}
        </button>
      </div>

      {/* progress bar */}
      <div className="h-[3px] w-full overflow-hidden bg-stone-200 dark:bg-stone-800">
        <div
          className="h-full bg-amber-500 transition-[width] duration-300 ease-out"
          style={{ width: scanning ? '66%' : settled ? '100%' : '0%' }}
        />
      </div>

      {/* live walker (scanning only) */}
      {scanning && (
        <div className="relative max-h-[172px] overflow-hidden border-t border-stone-200 px-3 pb-3 pt-1 dark:border-stone-800">
          <div className="pointer-events-none absolute inset-x-0 top-0 z-[2] h-4 bg-gradient-to-b from-stone-50 to-transparent dark:from-stone-950" />
          {walker.length === 0 ? (
            <div className="py-2 font-mono text-[11px] text-stone-400 dark:text-stone-600">
              Walking…
            </div>
          ) : (
            walker.map((row) => (
              <div
                key={row.id}
                className={`flex items-center gap-1.5 whitespace-nowrap font-mono text-[11px] leading-[1.85] ${ROW_CLASS[row.kind]}`}
              >
                <span className="w-3.5 shrink-0 text-center">{ROW_ICON[row.kind]}</span>
                <span className="truncate">
                  {row.path}
                  {row.kind === 'skip' ? '  (skipped)' : ''}
                </span>
              </div>
            ))
          )}
        </div>
      )}

      {/* result card (done / empty) */}
      {showResult && summary && (
        <div className="border-t border-stone-200 px-3 py-3 dark:border-stone-800">
          <div className="flex flex-wrap gap-4">
            <Stat num={summary.terms.toLocaleString()} cap="terms" />
            <Stat num={summary.files.toLocaleString()} cap="files read" />
            <Stat num={summary.skipped.toLocaleString()} cap="dirs skipped" />
            <Stat num={formatBytes(summary.bytes)} cap="scanned" />
            <Stat num={formatMs(summary.ms)} cap="duration" />
          </div>

          {/* cap warning steers toward a single project */}
          {summary.capped && (
            <WarnBox>
              Hit the <b>1,000-file cap</b> before finishing — some folders weren't indexed.
              Point at a single project (e.g. <b>/code/murmur-app</b>) for full coverage.
            </WarnBox>
          )}
          {/* empty result warning — distinguish "no files scanned" from
              "scanned files but no identifiers" (status==='empty' is terms===0). */}
          {!summary.capped && status === 'empty' && (
            <WarnBox>
              {summary.files > 0 ? (
                <>
                  Scanned <b>{summary.files} files</b> but found <b>0 identifiers</b> —
                  built-in dev terms still apply.
                </>
              ) : (
                <>
                  Found <b>0 source files</b>. Wrong folder, or everything was in skipped dirs
                  (node_modules, target, .git).
                </>
              )}
            </WarnBox>
          )}

          {/* sample-term chips */}
          {summary.sampleTerms.length > 0 && (
            <div className="mt-3">
              <div className="mb-1.5 text-[10px] uppercase tracking-wide text-stone-500 dark:text-stone-500">
                Top terms found
              </div>
              <div className="flex flex-wrap gap-1.5">
                {summary.sampleTerms.map((term) => (
                  <span
                    key={term}
                    className="rounded-md border border-stone-200 bg-stone-100 px-2 py-0.5 font-mono text-[11px] text-stone-700 dark:border-stone-700 dark:bg-stone-800 dark:text-stone-300"
                  >
                    {term}
                  </span>
                ))}
                {summary.terms > summary.sampleTerms.length && (
                  <span className="rounded-md px-2 py-0.5 font-mono text-[11px] text-stone-400 dark:text-stone-500">
                    +{summary.terms - summary.sampleTerms.length} more
                  </span>
                )}
                {/* View-all link: only when the backend returned a ranked list. */}
                {summary.rankedTerms.length > 0 && (
                  <button
                    type="button"
                    onClick={() => setModalOpen(true)}
                    className="ml-0.5 cursor-pointer text-[11px] font-semibold text-blue-500 underline hover:no-underline dark:text-blue-400"
                  >
                    View all {summary.rankedTerms.length} →
                  </button>
                )}
              </div>
            </div>
          )}
        </div>
      )}

      {/* View-all pop-out (over the panel). Only mountable once a ranked scan exists. */}
      {modalOpen && summary && summary.rankedTerms.length > 0 && (
        <VocabTermsModal
          summary={summary}
          folder={folder}
          onClose={() => setModalOpen(false)}
        />
      )}
    </div>
  );
}

function Stat({ num, cap }: { num: string; cap: string }) {
  return (
    <div className="flex flex-col">
      <span className="text-lg font-semibold tabular-nums text-stone-800 dark:text-stone-200">
        {num}
      </span>
      <span className="mt-px text-[10px] uppercase tracking-wide text-stone-500 dark:text-stone-500">
        {cap}
      </span>
    </div>
  );
}

function WarnBox({ children }: { children: React.ReactNode }) {
  return (
    <div className="mt-3 flex gap-2 rounded-lg border border-amber-500/25 bg-amber-500/10 px-3 py-2 text-[11px] leading-relaxed text-amber-600 dark:bg-amber-500/[0.08] dark:text-amber-500">
      <svg
        className="mt-px shrink-0"
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
      >
        <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
        <line x1="12" y1="9" x2="12" y2="13" />
        <line x1="12" y1="17" x2="12.01" y2="17" />
      </svg>
      <span>{children}</span>
    </div>
  );
}
