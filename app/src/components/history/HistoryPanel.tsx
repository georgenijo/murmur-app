import { useEffect, useId, useMemo, useRef, useState } from 'react';
import {
  HISTORY_EXPORT_FORMATS,
  HISTORY_FILTER_OPTIONS,
  entrySource,
  filterHistory,
  formatDuration,
  formatTimestamp,
  matchSegments,
  sortForDisplay,
  type HistoryEntry,
  type HistoryExportFormat,
  type HistoryFilter,
} from '../../lib/history';
import { copyHistoryExport, saveHistoryExport } from '../../lib/historyExport';
import { flog } from '../../lib/log';
import { CorrectAndTeachDialog } from './CorrectAndTeachDialog';

interface HistoryPanelProps {
  entries: HistoryEntry[];
  /** Clear the whole history. */
  onClear: () => void;
  onUpdateEntry: (id: string, text: string) => void;
  /** Bumped by the command palette to move focus into the search box. */
  focusSearchToken?: number;
  onTranscribeFile?: () => void;
}

function HighlightedText({ text, query }: { text: string; query: string }) {
  const segments = useMemo(() => matchSegments(text, query), [text, query]);
  return (
    <>
      {segments.map((segment, index) => segment.match ? (
        <mark key={index} className="rounded-sm bg-warning/10 px-0.5 text-warning">{segment.text}</mark>
      ) : (
        <span key={index}>{segment.text}</span>
      ))}
    </>
  );
}

function ClampedTranscript({ text, query }: { text: string; query: string }) {
  const textRef = useRef<HTMLParagraphElement>(null);
  const [expanded, setExpanded] = useState(false);
  const [overflows, setOverflows] = useState(false);

  useEffect(() => {
    const node = textRef.current;
    if (!node || expanded) return;
    const measure = () => setOverflows(node.scrollHeight > node.clientHeight + 1);
    measure();
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    observer?.observe(node);
    return () => observer?.disconnect();
  }, [expanded, text, query]);

  return (
    <>
      <p
        ref={textRef}
        className="text-[13px] leading-[1.55] text-on-surface"
        style={expanded ? undefined : {
          display: '-webkit-box',
          WebkitBoxOrient: 'vertical',
          WebkitLineClamp: 2,
          overflow: 'hidden',
        }}
      >
        <HighlightedText text={text} query={query} />
      </p>
      {(overflows || expanded) && (
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          className="mt-1 rounded px-0.5 py-0.5 text-[11px] font-semibold text-on-surface-variant hover:text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          {expanded ? 'Show less' : 'Show more'}
        </button>
      )}
    </>
  );
}

export function HistoryPanel({
  entries,
  onClear,
  onUpdateEntry,
  focusSearchToken,
  onTranscribeFile = () => {},
}: HistoryPanelProps) {
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [teachingEntry, setTeachingEntry] = useState<HistoryEntry | null>(null);
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<HistoryFilter>('all');
  const [exportOpen, setExportOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const copyGroupId = useId();
  const saveGroupId = useId();
  const exportPanelId = useId();
  const searchInputId = useId();
  const searchRef = useRef<HTMLInputElement>(null);
  const exportRef = useRef<HTMLDivElement>(null);
  const exportButtonRef = useRef<HTMLButtonElement>(null);
  const noticeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const confirmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => {
    if (noticeTimerRef.current) clearTimeout(noticeTimerRef.current);
    if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
    if (confirmTimerRef.current) clearTimeout(confirmTimerRef.current);
  }, []);

  useEffect(() => {
    if (focusSearchToken === undefined) return;
    searchRef.current?.focus();
    searchRef.current?.select();
  }, [focusSearchToken]);

  // Close the export menu on an outside click or Escape.
  useEffect(() => {
    if (!exportOpen) return;
    const onPointerDown = (event: MouseEvent) => {
      if (!exportRef.current?.contains(event.target as Node)) setExportOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setExportOpen(false);
        exportButtonRef.current?.focus();
      }
    };
    document.addEventListener('mousedown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [exportOpen]);

  const showNotice = (message: string) => {
    setNotice(message);
    if (noticeTimerRef.current) clearTimeout(noticeTimerRef.current);
    noticeTimerRef.current = setTimeout(() => setNotice(null), 4000);
  };

  const closeExportAndFocus = () => {
    exportButtonRef.current?.focus();
    setExportOpen(false);
  };

  const visible = useMemo(
    () => sortForDisplay(filterHistory(entries, { query, filter })),
    [entries, query, filter],
  );
  // Correct-and-Teach only ever targets the newest entry in the whole history,
  // not the first row on screen — sorting and filtering reorder the list.
  const newestId = entries[entries.length - 1]?.id;

  const handleCopy = async (entry: HistoryEntry) => {
    try {
      await navigator.clipboard.writeText(entry.text);
      setCopiedId(entry.id);
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
      copyTimerRef.current = setTimeout(() => setCopiedId(null), 2000);
    } catch (err) {
      showNotice('Could not copy to the clipboard.');
      flog.warn('main', 'History copy failed', { error: String(err) });
    }
  };

  const handleCopyExport = async (format: HistoryExportFormat) => {
    closeExportAndFocus();
    try {
      const count = await copyHistoryExport(visible, format);
      showNotice(`Copied ${count} ${count === 1 ? 'entry' : 'entries'} to the clipboard.`);
    } catch (err) {
      showNotice('Export to clipboard failed.');
      flog.warn('main', 'History export copy failed', { error: String(err) });
    }
  };

  const handleSaveExport = async (format: HistoryExportFormat) => {
    closeExportAndFocus();
    try {
      const path = await saveHistoryExport(visible, format);
      if (path) showNotice(`Saved ${visible.length} ${visible.length === 1 ? 'entry' : 'entries'}.`);
    } catch (err) {
      showNotice(`Could not save the export: ${String(err)}`);
      flog.warn('main', 'History export save failed', { error: String(err) });
    }
  };

  // Two-step confirm rather than window.confirm: the main window is a
  // non-activating utility surface and a native modal steals focus from it.
  const handleClear = () => {
    if (confirmTimerRef.current) clearTimeout(confirmTimerRef.current);
    if (!confirmClear) {
      setConfirmClear(true);
      confirmTimerRef.current = setTimeout(() => setConfirmClear(false), 4000);
      return;
    }
    setConfirmClear(false);
    onClear();
  };

  const closeSearch = () => {
    setQuery('');
  };

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <div className="mb-3 shrink-0 space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <div
            data-testid="history-search-shell"
            data-expanded="true"
            className="relative h-9 w-[min(25rem,52vw)] shrink-0 overflow-hidden rounded-xl border border-outline-variant/30 bg-surface-container-lowest shadow-sm transition-[border-color,box-shadow] focus-within:border-primary/70 focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--murmur-primary)_10%,transparent)]"
          >
            <span className="pointer-events-none absolute inset-y-0 left-0 z-10 grid w-9 place-items-center text-on-surface-variant">
              <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-4.35-4.35M17 11a6 6 0 11-12 0 6 6 0 0112 0z" />
              </svg>
            </span>
            <input
              id={searchInputId}
              ref={searchRef}
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key !== 'Escape') return;
                event.preventDefault();
                event.stopPropagation();
                closeSearch();
              }}
              placeholder="Search transcripts"
              aria-label="Search transcripts"
              className="absolute inset-0 h-full w-full border-0 bg-transparent py-1.5 pl-9 pr-10 text-sm text-on-surface outline-none placeholder:text-on-surface-variant [&::-webkit-search-cancel-button]:appearance-none"
            />
            {query ? (
              <button
                type="button"
                onClick={closeSearch}
                aria-label="Clear transcript search"
                className="absolute right-1.5 top-1.5 z-10 grid h-6 w-6 place-items-center rounded-md bg-on-surface/5 text-sm leading-none text-on-surface-variant transition-colors hover:bg-on-surface/10 hover:text-on-surface focus:outline-none focus-visible:ring-1 focus-visible:ring-primary"
              >
                ×
              </button>
            ) : (
              <kbd className="pointer-events-none absolute right-2 top-1.5 rounded-md bg-surface-container-high px-2 py-0.5 font-[inherit] text-[10px] text-on-surface-variant">/</kbd>
            )}
          </div>

          {HISTORY_FILTER_OPTIONS.map((option) => (
            <button
              key={option.value}
              type="button"
              aria-pressed={filter === option.value}
              onClick={() => setFilter(option.value)}
              className={`rounded-full px-3 py-1.5 text-xs font-semibold transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary ${filter === option.value ? 'bg-on-surface text-background' : 'bg-surface-container-low text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface'}`}
            >
              {option.label}
            </button>
          ))}

          <span className="ml-auto text-[11px] text-on-surface-variant">
            {visible.length === entries.length
              ? `${entries.length} ${entries.length === 1 ? 'entry' : 'entries'}`
              : `${visible.length} of ${entries.length}`}
          </span>

          <div ref={exportRef} className="relative shrink-0">
            <button
              ref={exportButtonRef}
              type="button"
              onClick={() => setExportOpen((open) => !open)}
              aria-expanded={exportOpen}
              aria-controls={exportPanelId}
              aria-label="More history actions"
              className="grid h-8 w-8 place-items-center rounded-lg text-lg leading-none text-on-surface-variant transition-colors hover:bg-surface-container-low hover:text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              ···
            </button>
            {exportOpen && (
              <div
                id={exportPanelId}
                role="group"
                aria-label="History actions"
                className="absolute right-0 z-20 mt-1 w-56 overflow-hidden rounded-xl bg-surface-container-lowest py-1.5 shadow-2xl ring-1 ring-outline-variant/30"
              >
                <button
                  type="button"
                  onClick={() => {
                    closeExportAndFocus();
                    onTranscribeFile();
                  }}
                  className="block w-full px-3 py-2 text-left text-xs font-medium text-on-surface hover:bg-surface-container"
                >
                  Transcribe audio file…
                </button>
                <div className="my-1 border-t border-outline-variant/20" />
                {/* Both groups render the same format names, so each group is
                    labelled and each item carries the verb explicitly —
                    otherwise a screen reader announces "Markdown" twice. */}
                <div role="group" aria-labelledby={copyGroupId}>
                  <p id={copyGroupId} className="px-3 py-1 text-[10px] font-semibold uppercase tracking-wide text-on-surface-variant">
                    Copy {visible.length} shown
                  </p>
                  {HISTORY_EXPORT_FORMATS.map((format) => (
                    <button
                      key={`copy-${format.value}`}
                      type="button"
                      aria-label={`Copy ${visible.length} shown as ${format.label}`}
                      onClick={() => void handleCopyExport(format.value)}
                      className="block w-full px-3 py-1.5 text-left text-xs text-on-surface hover:bg-surface-container"
                    >
                      {format.label}
                    </button>
                  ))}
                </div>
                <div className="my-1 border-t border-outline-variant/20" />
                <div role="group" aria-labelledby={saveGroupId}>
                  <p id={saveGroupId} className="px-3 py-1 text-[10px] font-semibold uppercase tracking-wide text-on-surface-variant">
                    Save to file
                  </p>
                  {HISTORY_EXPORT_FORMATS.map((format) => (
                    <button
                      key={`save-${format.value}`}
                      type="button"
                      aria-label={`Save ${visible.length} shown as ${format.label}`}
                      onClick={() => void handleSaveExport(format.value)}
                      className="block w-full px-3 py-1.5 text-left text-xs text-on-surface hover:bg-surface-container"
                    >
                      {format.label}…
                    </button>
                  ))}
                </div>
                <div className="my-1 border-t border-outline-variant/20" />
                <button
                  type="button"
                  onClick={handleClear}
                  disabled={entries.length === 0}
                  className={`block w-full px-3 py-2 text-left text-xs font-medium disabled:opacity-40 ${
                    confirmClear ? 'bg-error/10 text-error' : 'text-error hover:bg-error/10'
                  }`}
                >
                  {confirmClear ? 'Clear all history?' : 'Clear history'}
                </button>
              </div>
            )}
          </div>
        </div>

        {notice && (
          <p role="status" className="rounded-lg bg-surface-container px-2.5 py-1.5 text-[11px] text-on-surface-variant">{notice}</p>
        )}
      </div>

      <div className="flex-1 space-y-2 overflow-y-auto pr-1">
        {entries.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center text-on-surface-variant">
            <span className="mb-3 grid h-12 w-12 place-items-center rounded-2xl bg-surface-container-low text-xl">◌</span>
            <p className="text-sm font-medium text-on-surface">No transcription history yet</p>
            <p className="mt-1 text-xs">Your private, local transcripts will appear here.</p>
          </div>
        ) : visible.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-10 text-on-surface-variant">
            <p className="text-sm">No matching transcripts</p>
            <button type="button" onClick={() => { setQuery(''); setFilter('all'); }} className="mt-2 rounded-md px-2 py-1 text-xs font-medium text-on-surface hover:bg-surface-container">Reset filters</button>
          </div>
        ) : visible.map((entry) => {
          const wordCount = entry.text.trim() ? entry.text.trim().split(/\s+/).length : 0;
          const isNewest = entry.id === newestId;
          return (
            <article key={entry.id} className={`group w-full rounded-xl border px-3.5 py-3 text-left transition-[border-color,background-color] ${copiedId === entry.id ? 'border-success bg-surface-container-lowest' : 'border-outline-variant/25 bg-surface-container-lowest hover:border-outline-variant/45 hover:bg-surface-container-low'}`}>
              <div className="mb-1 flex items-center justify-between gap-2">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="shrink-0 text-xs text-on-surface-variant">{formatTimestamp(entry.timestamp)}</span>
                  {entrySource(entry) === 'file' ? (
                    <span title={entry.sourceName} className="inline-flex max-w-[180px] min-w-0 items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-on-surface">
                      <svg className="h-2.5 w-2.5 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z" /></svg>
                      <span className="truncate">{entry.sourceName || 'File'}</span>
                    </span>
                  ) : (
                    <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-surface-container px-2 py-0.5 text-[10px] font-medium text-on-surface-variant">
                      <svg className="h-2.5 w-2.5 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 11a7 7 0 01-14 0m7 7v3m-4 0h8m-4-6a3 3 0 01-3-3V5a3 3 0 016 0v4a3 3 0 01-3 3z" /></svg>
                      Mic
                    </span>
                  )}
                  {entry.interruption && (
                    <span
                      title={`Capture interrupted: ${entry.interruption.reason}`}
                      className="inline-flex shrink-0 items-center rounded-full bg-error-container px-2 py-0.5 text-[10px] font-semibold text-on-error-container"
                    >
                      Interrupted · partial
                    </span>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <span className="text-[11px] text-on-surface-variant">{wordCount} {wordCount === 1 ? 'word' : 'words'}</span>
                  <span className="text-xs text-on-surface-variant">{formatDuration(entry.duration)}</span>
                  {copiedId === entry.id ? (
                    <span className="text-xs font-medium text-success">Copied!</span>
                  ) : (
                    <button type="button" onClick={() => void handleCopy(entry)} aria-label={`Copy transcription from ${formatTimestamp(entry.timestamp)}`} className="rounded-md px-2 py-1 text-xs font-semibold text-on-surface-variant opacity-0 transition-opacity hover:bg-surface-container hover:text-on-surface focus:opacity-100 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary group-hover:opacity-100">Copy</button>
                  )}
                </div>
              </div>
              <ClampedTranscript text={entry.text} query={query} />
              {isNewest && (
                <div className="mt-2">
                  <button type="button" onClick={() => setTeachingEntry(entry)} className="rounded-md bg-surface-container-high px-2.5 py-1 text-[11px] font-semibold text-on-surface hover:bg-primary/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">Correct &amp; Teach</button>
                </div>
              )}
            </article>
          );
        })}
      </div>

      {teachingEntry && (
        <CorrectAndTeachDialog
          entry={teachingEntry}
          onClose={() => setTeachingEntry(null)}
          onSaveCorrection={(text) => onUpdateEntry(teachingEntry.id, text)}
        />
      )}
    </div>
  );
}
