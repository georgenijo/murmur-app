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

export function HistoryPanel({
  entries,
  onClear,
  onUpdateEntry,
  focusSearchToken,
}: HistoryPanelProps) {
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [teachingEntry, setTeachingEntry] = useState<HistoryEntry | null>(null);
  const [query, setQuery] = useState('');
  const [searchHovered, setSearchHovered] = useState(false);
  const [searchPinned, setSearchPinned] = useState(false);
  const [searchHoverSuppressed, setSearchHoverSuppressed] = useState(false);
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
    setSearchHoverSuppressed(false);
    setSearchPinned(true);
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
  const searchExpanded = searchPinned
    || query.length > 0
    || (searchHovered && !searchHoverSuppressed);
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

  const openSearch = () => {
    setSearchHoverSuppressed(false);
    setSearchPinned(true);
    searchRef.current?.focus();
  };

  const closeSearch = () => {
    setQuery('');
    setSearchPinned(false);
    // Escape/close must win if the pointer is still sitting on the control,
    // but a keyboard close elsewhere must not suppress the next fresh hover.
    setSearchHoverSuppressed(searchHovered);
    searchRef.current?.blur();
  };

  if (entries.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center text-on-surface-variant">
        <svg className="mb-3 h-12 w-12" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
        <p className="text-sm">No transcription history yet</p>
        <p className="mt-1 text-xs">Your transcriptions will appear here</p>
      </div>
    );
  }

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <div className="mb-2 shrink-0 space-y-2">
        <div className="flex flex-wrap items-center gap-1.5">
          <div
            data-testid="history-search-shell"
            data-expanded={searchExpanded}
            className={`relative h-8 shrink-0 overflow-hidden rounded-lg border bg-surface-container-lowest shadow-sm motion-safe:transition-[width,border-color,background-color,box-shadow] motion-safe:duration-300 motion-safe:ease-[cubic-bezier(0.2,0.86,0.24,1.1)] motion-reduce:transition-none ${
              searchExpanded
                ? 'w-[min(24rem,55vw)] border-primary/70 bg-surface-container shadow-[0_0_0_3px_color-mix(in_srgb,var(--color-primary)_10%,transparent)]'
                : 'w-8 border-outline-variant/30'
            }`}
            onMouseEnter={() => setSearchHovered(true)}
            onMouseLeave={() => {
              setSearchHovered(false);
              setSearchHoverSuppressed(false);
            }}
            onBlur={(event) => {
              if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
              setSearchPinned(false);
            }}
          >
            <button
              type="button"
              onClick={openSearch}
              aria-label="Open transcript search"
              aria-expanded={searchExpanded}
              aria-controls={searchInputId}
              className="absolute inset-y-0 left-0 z-10 grid w-8 place-items-center rounded-lg text-on-surface-variant transition-colors hover:text-primary focus:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-primary"
            >
              <svg className={`h-3.5 w-3.5 motion-safe:transition-[color,transform] motion-safe:duration-300 motion-reduce:transition-none ${searchExpanded ? 'scale-95 text-primary' : ''}`} fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-4.35-4.35M17 11a6 6 0 11-12 0 6 6 0 0112 0z" />
              </svg>
            </button>
            <input
              id={searchInputId}
              ref={searchRef}
              type="search"
              value={query}
              onFocus={() => setSearchPinned(true)}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key !== 'Escape') return;
                event.preventDefault();
                event.stopPropagation();
                closeSearch();
              }}
              placeholder="Search transcripts"
              aria-label="Search transcripts"
              aria-hidden={!searchExpanded}
              tabIndex={searchExpanded ? 0 : -1}
              className={`absolute inset-0 h-full w-full border-0 bg-transparent py-1.5 pl-8 pr-8 text-sm text-on-surface outline-none placeholder:text-on-surface-variant [&::-webkit-search-cancel-button]:appearance-none motion-safe:transition-[opacity,transform] motion-safe:duration-200 motion-reduce:transition-none ${
                searchExpanded
                  ? 'translate-x-0 opacity-100'
                  : '-translate-x-1.5 opacity-0 pointer-events-none'
              }`}
            />
            <button
              type="button"
              onClick={closeSearch}
              aria-label="Close transcript search"
              aria-hidden={!searchExpanded}
              tabIndex={searchExpanded ? 0 : -1}
              className={`absolute right-1 top-1 z-10 grid h-6 w-6 place-items-center rounded-md bg-on-surface/5 text-sm leading-none text-on-surface-variant transition-[opacity,color,background-color] hover:bg-on-surface/10 hover:text-on-surface focus:outline-none focus-visible:ring-1 focus-visible:ring-primary ${
                searchExpanded ? 'opacity-100' : 'pointer-events-none opacity-0'
              }`}
            >
              ×
            </button>
          </div>

          {HISTORY_FILTER_OPTIONS.map((option) => (
            <button
              key={option.value}
              type="button"
              aria-pressed={filter === option.value}
              onClick={() => setFilter(option.value)}
              className={`rounded-full px-2.5 py-1 text-[11px] font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary ${filter === option.value ? 'bg-primary text-on-primary' : 'bg-surface-container-lowest text-on-surface-variant hover:bg-surface-container'}`}
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
              disabled={visible.length === 0}
              className="rounded-lg bg-surface-container-lowest px-2.5 py-1.5 text-xs font-medium text-on-surface-variant shadow-sm transition-colors hover:bg-surface-container hover:text-on-surface disabled:cursor-not-allowed disabled:opacity-40 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              Export ▾
            </button>
            {exportOpen && (
              <div
                id={exportPanelId}
                role="group"
                aria-label="History export actions"
                className="absolute right-0 z-20 mt-1 w-52 overflow-hidden rounded-xl bg-surface-container-lowest py-1 shadow-lg ring-1 ring-outline-variant/30"
              >
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
              </div>
            )}
          </div>
        </div>

        {notice && (
          <p role="status" className="rounded-lg bg-surface-container px-2.5 py-1.5 text-[11px] text-on-surface-variant">{notice}</p>
        )}
      </div>

      <div className="flex-1 space-y-3 overflow-y-auto px-0.5 py-0.5">
        {visible.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-10 text-on-surface-variant">
            <p className="text-sm">No matching transcripts</p>
            <button type="button" onClick={() => { setQuery(''); setFilter('all'); }} className="mt-2 rounded-md px-2 py-1 text-xs font-medium text-on-surface hover:bg-surface-container">Reset filters</button>
          </div>
        ) : visible.map((entry) => {
          const wordCount = entry.text.trim() ? entry.text.trim().split(/\s+/).length : 0;
          const isNewest = entry.id === newestId;
          return (
            <article key={entry.id} className={`group w-full rounded-xl border p-3.5 text-left shadow-sm transition-[box-shadow,background-color] hover:shadow-md ${copiedId === entry.id ? 'border-success bg-surface-container-lowest' : 'border-outline-variant/30 bg-surface-container-lowest hover:bg-surface-container-low'}`}>
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
                  <span className="rounded-full bg-surface-container px-2 py-0.5 text-[10px] font-medium text-on-surface-variant">{wordCount} {wordCount === 1 ? 'word' : 'words'}</span>
                  <span className="text-xs text-on-surface-variant">{formatDuration(entry.duration)}</span>
                  {copiedId === entry.id ? (
                    <span className="text-xs font-medium text-success">Copied!</span>
                  ) : (
                    <button type="button" onClick={() => void handleCopy(entry)} aria-label={`Copy transcription from ${formatTimestamp(entry.timestamp)}`} className="rounded-md px-2 py-1 text-xs font-medium text-on-surface-variant hover:bg-surface-container hover:text-primary focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">Copy</button>
                  )}
                </div>
              </div>
              <p className="max-h-32 overflow-y-auto text-sm leading-relaxed text-on-surface">
                <HighlightedText text={entry.text} query={query} />
              </p>
              {isNewest && (
                <div className="mt-3 border-t border-outline-variant/20 pt-2">
                  <button type="button" onClick={() => setTeachingEntry(entry)} className="rounded-md px-2 py-1 text-xs font-semibold text-on-surface hover:bg-primary/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">Correct and Teach</button>
                </div>
              )}
            </article>
          );
        })}
      </div>

      <div className="mt-3 flex shrink-0 gap-2 pt-1">
        <button
          onClick={handleClear}
          className="flex-1 rounded-lg bg-surface-container-lowest px-3 py-2 text-sm font-medium text-on-surface-variant shadow-sm transition-colors hover:bg-surface-container hover:text-error focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          {confirmClear ? 'Click again to confirm' : 'Clear History'}
        </button>
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
