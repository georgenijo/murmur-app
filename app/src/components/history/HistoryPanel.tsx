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
        <mark key={index} className="rounded-sm bg-amber-200/70 px-0.5 text-on-surface dark:bg-amber-500/30">{segment.text}</mark>
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
  const [filter, setFilter] = useState<HistoryFilter>('all');
  const [exportOpen, setExportOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const copyGroupId = useId();
  const saveGroupId = useId();
  const searchRef = useRef<HTMLInputElement>(null);
  const exportRef = useRef<HTMLDivElement>(null);
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
      if (event.key === 'Escape') setExportOpen(false);
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
    setExportOpen(false);
    try {
      const count = await copyHistoryExport(visible, format);
      showNotice(`Copied ${count} ${count === 1 ? 'entry' : 'entries'} to the clipboard.`);
    } catch (err) {
      showNotice('Export to clipboard failed.');
      flog.warn('main', 'History export copy failed', { error: String(err) });
    }
  };

  const handleSaveExport = async (format: HistoryExportFormat) => {
    setExportOpen(false);
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
        <div className="flex items-center gap-2">
          <div className="relative min-w-0 flex-1">
            <svg className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-4.35-4.35M17 11a6 6 0 11-12 0 6 6 0 0112 0z" />
            </svg>
            <input
              ref={searchRef}
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => { if (event.key === 'Escape' && query) { event.stopPropagation(); setQuery(''); } }}
              placeholder="Search transcripts"
              aria-label="Search transcripts"
              className="w-full rounded-lg bg-surface-container-lowest py-1.5 pl-8 pr-2 text-sm text-on-surface shadow-sm placeholder:text-on-surface-variant focus:outline-none focus-visible:ring-2 focus-visible:ring-primary [&::-webkit-search-cancel-button]:appearance-none"
            />
          </div>
          <div ref={exportRef} className="relative shrink-0">
            <button
              type="button"
              onClick={() => setExportOpen((open) => !open)}
              aria-haspopup="menu"
              aria-expanded={exportOpen}
              disabled={visible.length === 0}
              className="rounded-lg bg-surface-container-lowest px-2.5 py-1.5 text-xs font-medium text-on-surface-variant shadow-sm transition-colors hover:bg-surface-container hover:text-on-surface disabled:cursor-not-allowed disabled:opacity-40 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              Export ▾
            </button>
            {exportOpen && (
              <div role="menu" className="absolute right-0 z-20 mt-1 w-52 overflow-hidden rounded-xl bg-surface-container-lowest py-1 shadow-lg ring-1 ring-outline-variant/30">
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
                      role="menuitem"
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
                      role="menuitem"
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

        <div className="flex items-center gap-1.5">
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
        </div>

        {notice && (
          <p role="status" className="rounded-lg bg-surface-container px-2.5 py-1.5 text-[11px] text-on-surface-variant">{notice}</p>
        )}
      </div>

      <div className="flex-1 space-y-3 overflow-y-auto px-0.5 py-0.5">
        {visible.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-10 text-on-surface-variant">
            <p className="text-sm">No matching transcripts</p>
            <button type="button" onClick={() => { setQuery(''); setFilter('all'); }} className="mt-2 rounded-md px-2 py-1 text-xs font-medium text-primary hover:bg-primary/10">Reset filters</button>
          </div>
        ) : visible.map((entry) => {
          const wordCount = entry.text.trim() ? entry.text.trim().split(/\s+/).length : 0;
          const isNewest = entry.id === newestId;
          return (
            <article key={entry.id} className={`group w-full rounded-xl p-3.5 text-left shadow-sm transition-[box-shadow,background-color] hover:shadow-md ${copiedId === entry.id ? 'bg-emerald-50 dark:bg-emerald-950/40' : 'bg-surface-container-lowest hover:bg-surface-container-low'}`}>
              <div className="mb-1 flex items-center justify-between gap-2">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="shrink-0 text-xs text-on-surface-variant">{formatTimestamp(entry.timestamp)}</span>
                  {entrySource(entry) === 'file' ? (
                    <span title={entry.sourceName} className="inline-flex max-w-[180px] min-w-0 items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary">
                      <svg className="h-2.5 w-2.5 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z" /></svg>
                      <span className="truncate">{entry.sourceName || 'File'}</span>
                    </span>
                  ) : (
                    <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-surface-container px-2 py-0.5 text-[10px] font-medium text-on-surface-variant">
                      <svg className="h-2.5 w-2.5 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 11a7 7 0 01-14 0m7 7v3m-4 0h8m-4-6a3 3 0 01-3-3V5a3 3 0 016 0v4a3 3 0 01-3 3z" /></svg>
                      Mic
                    </span>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <span className="rounded-full bg-surface-container px-2 py-0.5 text-[10px] font-medium text-on-surface-variant">{wordCount} {wordCount === 1 ? 'word' : 'words'}</span>
                  <span className="text-xs text-on-surface-variant">{formatDuration(entry.duration)}</span>
                  {copiedId === entry.id ? (
                    <span className="text-xs font-medium text-emerald-600 dark:text-emerald-400">Copied!</span>
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
                  <button type="button" onClick={() => setTeachingEntry(entry)} className="rounded-md px-2 py-1 text-xs font-semibold text-primary hover:bg-primary/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">Correct and Teach</button>
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
