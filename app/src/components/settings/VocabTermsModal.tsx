import { useEffect, useMemo, useRef, useState } from 'react';
import type { RankedTerm, VocabScanSummary } from '../../lib/settings';

interface VocabTermsModalProps {
  /** The completed scan whose ranked terms are shown. */
  summary: VocabScanSummary;
  /** Absolute path of the scanned folder, shown in the subheader. */
  folder: string;
  onClose: () => void;
  /** Explicit opener for reliable focus restoration after AX-initiated clicks. */
  returnFocusRef?: React.RefObject<HTMLElement>;
}

type SortMode = 'freq' | 'alpha';

/**
 * View-all pop-out for a code-vocab scan. Shows the full ranked term list
 * (<=500) with a search filter and a frequency<->A-Z sort toggle. The top
 * `whisperCount` terms feed Whisper's token-bound prompt (blue); the rest are
 * Smart-Correction-only (green). When sorted by frequency without a search
 * query, a sticky divider splits the two sections.
 *
 * Closes on Escape, the close button, and a click on the backdrop.
 */
export function VocabTermsModal({
  summary,
  folder,
  onClose,
  returnFocusRef,
}: VocabTermsModalProps) {
  const [query, setQuery] = useState('');
  const [sortMode, setSortMode] = useState<SortMode>('freq');
  const [copied, setCopied] = useState(false);
  const dialogRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const copyResetRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const ranked = summary.rankedTerms;
  const whisperCount = summary.whisperCount;

  // VocabScanStrip passes a fresh onClose closure each render. Stash it in a ref
  // so the keydown/focus effect can attach exactly once for the modal's lifetime
  // (empty deps) without tearing down the listener / rescheduling the focus timer
  // on every parent re-render.
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  // Keep keyboard focus inside the dialog, close on Escape, and restore the
  // element that opened it after unmount.
  useEffect(() => {
    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onCloseRef.current();
        return;
      }
      if (e.key !== 'Tab') return;

      const dialog = dialogRef.current;
      if (!dialog) return;
      const focusable = Array.from(
        dialog.querySelectorAll<HTMLElement>(
          'button:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((element) => !element.hasAttribute('hidden'));
      if (focusable.length === 0) {
        e.preventDefault();
        dialog.focus();
        return;
      }

      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (!dialog.contains(active)) {
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
      } else if (e.shiftKey && active === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener('keydown', onKey);
    const t = setTimeout(() => searchRef.current?.focus(), 60);
    return () => {
      document.removeEventListener('keydown', onKey);
      clearTimeout(t);
      (returnFocusRef?.current ?? previouslyFocused)?.focus();
    };
  }, [returnFocusRef]);

  // Clean up the "Copied" reset timer on unmount.
  useEffect(
    () => () => {
      if (copyResetRef.current) clearTimeout(copyResetRef.current);
    },
    [],
  );

  // Each ranked term carries its 1-based rank from the *original* frequency
  // order (array index + 1) — that's what the feed-color and divider depend on,
  // so it must survive re-sorting and filtering.
  const withRank = useMemo(
    () => ranked.map((t, i) => ({ ...t, rank: i + 1 })),
    [ranked],
  );

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    let list = q ? withRank.filter((t) => t.term.toLowerCase().includes(q)) : withRank;
    if (sortMode === 'alpha') {
      list = [...list].sort((a, b) => a.term.localeCompare(b.term));
    }
    return list;
  }, [withRank, query, sortMode]);

  // The sticky section dividers only make sense in frequency order with no
  // active filter (otherwise rows aren't contiguous by section).
  const showDividers = sortMode === 'freq' && query.trim() === '';

  const handleCopyAll = () => {
    const text = withRank.map((t) => t.term).join('\n');
    void navigator.clipboard?.writeText(text);
    setCopied(true);
    if (copyResetRef.current) clearTimeout(copyResetRef.current);
    copyResetRef.current = setTimeout(() => setCopied(false), 1200);
  };

  const countLabel = `${rows.length} ${rows.length === 1 ? 'term' : 'terms'}${
    query.trim() ? ` matching "${query.trim()}"` : ''
  }`;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-6 backdrop-blur-[2px]"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="vocab-terms-title"
        tabIndex={-1}
        className="flex max-h-[82vh] w-full max-w-[560px] flex-col overflow-hidden rounded-2xl border border-outline-variant/40 bg-surface-container-lowest shadow-2xl"
      >
        {/* header */}
        <div className="border-b border-outline-variant/40 px-[18px] pb-3 pt-4">
          <div className="flex items-center justify-between">
            <div
              id="vocab-terms-title"
              className="text-[15px] font-semibold text-on-surface"
            >
              Scanned vocabulary
            </div>
            <button
              type="button"
              onClick={onClose}
              aria-label="Close"
              className="flex h-[26px] w-[26px] items-center justify-center rounded-md border border-outline-variant/40 text-sm leading-none text-on-surface-variant transition-colors hover:border-outline-variant/40 hover:text-on-surface"
            >
              ✕
            </button>
          </div>
          <div className="mt-1 truncate text-[11px] text-on-surface-variant">
            {folder ? `${folder} · ` : ''}
            {ranked.length} identifiers ranked by frequency
          </div>
          <div className="mt-2.5 flex flex-wrap gap-2">
            <span className="flex items-center gap-1.5 rounded-md border border-primary/40 px-2 py-[3px] text-[10px] text-primary">
              <span className="h-2 w-2 rounded-sm bg-primary" />
              top {whisperCount} → Whisper prompt
            </span>
            <span className="flex items-center gap-1.5 rounded-md border border-success/35 px-2 py-[3px] text-[10px] text-success">
              <span className="h-2 w-2 rounded-sm bg-success" />
              all {ranked.length} → Smart Correction
            </span>
          </div>
        </div>

        {/* tools: search + sort */}
        <div className="flex items-center gap-2.5 border-b border-outline-variant/40 px-[18px] py-[11px]">
          <div className="flex flex-1 items-center gap-2 rounded-lg border border-on-surface-variant bg-surface-container-low px-[11px] py-[7px]">
            <svg
              className="shrink-0 text-on-surface-variant"
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
            >
              <circle cx="11" cy="11" r="7" />
              <path d="m21 21-4.3-4.3" />
            </svg>
            <input
              ref={searchRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter terms…"
              aria-label="Filter scanned terms"
              autoComplete="off"
              spellCheck={false}
              className="min-w-0 flex-1 border-none bg-transparent text-xs text-on-surface outline-none placeholder:text-on-surface-variant"
            />
          </div>
          <select
            value={sortMode}
            onChange={(e) => setSortMode(e.target.value as SortMode)}
            className="cursor-pointer rounded-lg border border-on-surface-variant bg-surface-container-low px-2.5 py-[7px] text-[11px] text-on-surface-variant"
          >
            <option value="freq">Sort: frequency</option>
            <option value="alpha">Sort: A–Z</option>
          </select>
        </div>

        {/* list */}
        <div className="flex-1 overflow-y-auto py-1.5">
          {rows.length === 0 ? (
            <div className="px-[18px] py-8 text-center text-xs text-on-surface-variant">
              No terms match.
            </div>
          ) : (
            <TermRows rows={rows} whisperCount={whisperCount} showDividers={showDividers} />
          )}
        </div>

        {/* footer */}
        <div className="flex items-center justify-between border-t border-outline-variant/40 px-[18px] py-2.5 text-[11px] text-on-surface-variant">
          <span>{countLabel}</span>
          <button
            type="button"
            onClick={handleCopyAll}
            className="rounded-md border border-outline-variant/40 bg-surface-container-lowest px-3 py-1.5 text-[11px] font-semibold text-on-surface-variant transition-colors hover:border-outline-variant/40"
          >
            {copied ? 'Copied ✓' : 'Copy all'}
          </button>
        </div>
      </div>
    </div>
  );
}

type RankedRow = RankedTerm & { rank: number };

/**
 * Renders the term rows, inserting a sticky section divider before the first
 * Whisper row and before the first correction-only row (only when
 * `showDividers` — i.e. frequency-sorted and unfiltered).
 */
function TermRows({
  rows,
  whisperCount,
  showDividers,
}: {
  rows: RankedRow[];
  whisperCount: number;
  showDividers: boolean;
}) {
  const total = rows.length;
  let whisperHdrDone = false;
  let belowHdrDone = false;
  const out: React.ReactNode[] = [];

  for (const t of rows) {
    const inWhisper = t.rank <= whisperCount;
    if (showDividers) {
      if (inWhisper && !whisperHdrDone) {
        whisperHdrDone = true;
        out.push(
          <Divider
            key="hdr-whisper"
            label={`Feeds Whisper prompt · top ${whisperCount}`}
            tone="whisper"
          />,
        );
      }
      if (!inWhisper && !belowHdrDone) {
        belowHdrDone = true;
        out.push(
          <Divider
            key="hdr-below"
            label={`Smart Correction only · ${whisperCount + 1}–${total}`}
            tone="below"
          />,
        );
      }
    }
    out.push(<TermRow key={`${t.rank}-${t.term}`} row={t} inWhisper={inWhisper} />);
  }

  return <>{out}</>;
}

function Divider({ label, tone }: { label: string; tone: 'whisper' | 'below' }) {
  return (
    <div
      className={`sticky top-0 z-[1] flex items-center gap-2 bg-surface-container-lowest px-[18px] pb-1.5 pt-2 text-[10px] uppercase tracking-wider ${
        tone === 'whisper'
          ? 'text-primary '
          : 'text-on-surface-variant '
      }`}
    >
      <span>{label}</span>
      <span className="h-px flex-1 bg-surface-container-high" />
    </div>
  );
}

function TermRow({ row, inWhisper }: { row: RankedRow; inWhisper: boolean }) {
  return (
    <div className="flex items-center gap-2.5 px-[18px] py-[5px] text-xs hover:bg-surface-container-low">
      <span className="w-[30px] shrink-0 text-right text-[10px] tabular-nums text-on-surface-variant">
        {row.rank}
      </span>
      <span
        className={`h-2 w-2 shrink-0 rounded-sm ${
          inWhisper ? 'bg-primary ' : 'bg-success opacity-60'
        }`}
      />
      <span
        className={`flex-1 truncate font-mono ${
          inWhisper ? 'text-on-surface ' : 'text-on-surface-variant '
        }`}
      >
        {row.term}
      </span>
      <span className="shrink-0 text-[10px] tabular-nums text-on-surface-variant">
        {row.freq}×
      </span>
    </div>
  );
}
