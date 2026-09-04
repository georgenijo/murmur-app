import { Fragment, memo, useEffect, useId, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react';
import { Copy, GraduationCap } from 'lucide-react';
import {
  HISTORY_EXPORT_FORMATS,
  HISTORY_FILTER_OPTIONS,
  entrySource,
  filterHistory,
  formatTimestamp,
  matchSegments,
  sortForDisplay,
  type HistoryEntry,
  type HistoryExportFormat,
  type HistoryFilter,
} from '../../lib/history';
import { copyHistoryExport, saveHistoryExport } from '../../lib/historyExport';
import { flog } from '../../lib/log';
import {
  AnimatedDropdown,
  AnimatedDropdownContent,
  AnimatedDropdownItem,
  AnimatedDropdownSeparator,
  AnimatedDropdownTrigger,
} from '@/components/ui/animated-dropdown/animated-dropdown';
import FluidTabs from '@/components/ui/fluid-tabs/fluid-tabs';
import HoldToDeleteButton from '@/components/ui/hold-to-delete-button/hold-to-delete-button';
import SmartOverflow, { SmartOverflowAction } from '@/components/ui/smart-overflow/smart-overflow';
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

const HISTORY_RENDER_BATCH = 30;
const HISTORY_FILTER_TABS = HISTORY_FILTER_OPTIONS.map((option) => ({
  value: option.value,
  title: option.label,
}));

function isHistoryFilter(value: string): value is HistoryFilter {
  return HISTORY_FILTER_OPTIONS.some((option) => option.value === value);
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

function historyDayKey(timestamp: number): string {
  const date = new Date(timestamp);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

function historyDayLabel(timestamp: number): string {
  const date = new Date(timestamp);
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  const key = historyDayKey(timestamp);
  if (key === historyDayKey(today.getTime())) return 'Today';
  if (key === historyDayKey(yesterday.getTime())) return 'Yesterday';
  return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
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
        className="transcript-text"
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
          onClick={(event) => {
            event.stopPropagation();
            setExpanded((value) => !value);
          }}
          className="mt-0.5 rounded px-0.5 py-0.5 text-xs font-semibold text-on-surface-variant hover:text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          {expanded ? 'Show less' : 'Show more'}
        </button>
      )}
    </>
  );
}

function HistoryPanelComponent({
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
  const [renderLimit, setRenderLimit] = useState(HISTORY_RENDER_BATCH);
  const searchInputId = useId();
  const searchRef = useRef<HTMLInputElement>(null);
  const noticeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => {
    if (noticeTimerRef.current) clearTimeout(noticeTimerRef.current);
    if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
  }, []);

  useEffect(() => {
    if (focusSearchToken === undefined) return;
    searchRef.current?.focus();
    searchRef.current?.select();
  }, [focusSearchToken]);

  const showNotice = (message: string) => {
    setNotice(message);
    if (noticeTimerRef.current) clearTimeout(noticeTimerRef.current);
    noticeTimerRef.current = setTimeout(() => setNotice(null), 4000);
  };

  const visible = useMemo(
    () => sortForDisplay(filterHistory(entries, { query, filter })),
    [entries, query, filter],
  );
  const rendered = useMemo(
    () => visible.slice(0, renderLimit),
    [visible, renderLimit],
  );
  useEffect(() => {
    setRenderLimit(HISTORY_RENDER_BATCH);
  }, [query, filter]);
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

  const handleCardClick = (event: ReactMouseEvent<HTMLElement>, entry: HistoryEntry) => {
    if (event.target !== event.currentTarget && (event.target as HTMLElement).closest('button, a, input, select, textarea')) return;
    void handleCopy(entry);
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

  const closeSearch = () => {
    setQuery('');
  };

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <div className="shrink-0">
        <div className="history-toolbar">
          <div
            data-testid="history-search-shell"
            data-expanded="true"
            className="history-search"
          >
            <span className="pointer-events-none absolute inset-y-0 left-0 z-10 grid w-7 place-items-center text-on-surface-variant">
              <svg className="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
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
              className="absolute inset-0 h-full w-full border-0 bg-transparent py-1 pl-7 pr-7 text-sm text-on-surface outline-none placeholder:text-on-surface-variant [&::-webkit-search-cancel-button]:appearance-none"
            />
            {query ? (
              <button
                type="button"
                onClick={closeSearch}
                aria-label="Clear transcript search"
                className="absolute right-1 top-1 z-10 grid h-5 w-5 place-items-center rounded bg-on-surface/5 text-xs leading-none text-on-surface-variant transition-colors hover:bg-on-surface/10 hover:text-on-surface focus:outline-none focus-visible:ring-1 focus-visible:ring-primary"
              >
                ×
              </button>
            ) : (
              <kbd className="pointer-events-none absolute right-2 top-1 rounded-full bg-on-surface/6 px-1 py-0.5 font-mono text-[9px] text-on-surface-variant">/</kbd>
            )}
          </div>

          <FluidTabs
            tabs={HISTORY_FILTER_TABS}
            value={filter}
            onValueChange={(value) => {
              if (isHistoryFilter(value)) setFilter(value);
            }}
            variant="capsule"
            size="sm"
            ariaLabel="Filter transcripts"
            className="history-filter-tabs"
            listClassName="history-filter-tabs-list"
            activeIndicatorClassName="history-filter-tab-indicator"
          />

          <span className="ml-auto text-xs tabular-nums text-on-surface-variant">
            {visible.length === entries.length
              ? `${entries.length} ${entries.length === 1 ? 'entry' : 'entries'}`
              : `${visible.length} of ${entries.length}`}
          </span>

          <AnimatedDropdown open={exportOpen} onOpenChange={setExportOpen}>
            <AnimatedDropdownTrigger className="history-export-trigger" aria-label="More history actions">
              <span aria-hidden="true" className="text-base leading-none">···</span>
            </AnimatedDropdownTrigger>
            <AnimatedDropdownContent align="end" side="bottom" className="history-export-menu min-w-56">
              <AnimatedDropdownItem
                onClick={() => {
                  setExportOpen(false);
                  onTranscribeFile();
                }}
              >
                Transcribe audio file…
              </AnimatedDropdownItem>
              <AnimatedDropdownSeparator />
              <p className="history-export-label">
                Copy {visible.length} shown
              </p>
              {HISTORY_EXPORT_FORMATS.map((format) => (
                <AnimatedDropdownItem
                  key={`copy-${format.value}`}
                  className="history-export-item"
                  aria-label={`Copy ${visible.length} shown as ${format.label}`}
                  onClick={() => void handleCopyExport(format.value)}
                >
                  {format.label}
                </AnimatedDropdownItem>
              ))}
              <AnimatedDropdownSeparator />
              <p className="history-export-label">Save to file</p>
              {HISTORY_EXPORT_FORMATS.map((format) => (
                <AnimatedDropdownItem
                  key={`save-${format.value}`}
                  className="history-export-item"
                  aria-label={`Save ${visible.length} shown as ${format.label}`}
                  onClick={() => void handleSaveExport(format.value)}
                >
                  {format.label}…
                </AnimatedDropdownItem>
              ))}
              <AnimatedDropdownSeparator />
              <HoldToDeleteButton
                label="Hold to clear history"
                confirmLabel="Click again to clear history"
                holdDuration={1200}
                onDelete={() => {
                  onClear();
                  setExportOpen(false);
                }}
                disabled={entries.length === 0}
                className="h-9 w-full min-w-0 justify-start rounded-lg px-2.5 text-[length:var(--ui-font-label)]"
              />
            </AnimatedDropdownContent>
          </AnimatedDropdown>
        </div>

        {notice && (
          <p role="status" className="mx-3.5 mb-1.5 rounded-lg bg-surface-container px-2.5 py-1.5 text-[11px] text-on-surface-variant">{notice}</p>
        )}
      </div>

      <div className="history-list">
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
        ) : rendered.map((entry, index) => {
          const isNewest = entry.id === newestId;
          const showDayLabel = index === 0 || historyDayKey(entry.timestamp) !== historyDayKey(rendered[index - 1].timestamp);
          const endsDay = index === visible.length - 1 || historyDayKey(entry.timestamp) !== historyDayKey(visible[index + 1].timestamp);
          return (
            <Fragment key={entry.id}>
              {showDayLabel && <p className="history-date-label">{historyDayLabel(entry.timestamp)}</p>}
              <article
                data-testid="transcript-card"
                data-copied={copiedId === entry.id}
                data-day-end={endsDay}
                data-newest={isNewest}
                role="group"
                tabIndex={0}
                aria-label={`Transcription from ${formatTimestamp(entry.timestamp)}. Press Enter or Space to copy.`}
                onClick={(event) => handleCardClick(event, entry)}
                onKeyDown={(event) => {
                  if (event.target !== event.currentTarget || (event.key !== 'Enter' && event.key !== ' ')) return;
                  event.preventDefault();
                  void handleCopy(entry);
                }}
                className="transcript-card group"
              >
              <div className="transcript-meta">
                <div className="flex min-w-0 items-center gap-1.5">
                  <span className="shrink-0">{formatTimestamp(entry.timestamp)}</span>
                  {entrySource(entry) === 'file' ? (
                    <span title={entry.sourceName} className="inline-flex max-w-[120px] min-w-0 items-center gap-0.5 rounded-full bg-primary/11 px-1.5 text-[10.5px] font-semibold text-primary">
                      <svg className="h-2 w-2 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z" /></svg>
                      <span className="truncate">{entry.sourceName || 'File'}</span>
                    </span>
                  ) : (
                    <span className="inline-flex shrink-0 items-center gap-0.5 rounded-full bg-on-surface/6 px-1.5 text-[10.5px] font-semibold text-on-surface-variant">
                      <svg className="h-2 w-2 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 11a7 7 0 01-14 0m7 7v3m-4 0h8m-4-6a3 3 0 01-3-3V5a3 3 0 016 0v4a3 3 0 01-3 3z" /></svg>
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
              </div>
              <ClampedTranscript text={entry.text} query={query} />
              <span className="transcript-copy-feedback" role="status" aria-live="polite">
                {copiedId === entry.id ? 'Copied' : ''}
              </span>
              <div className="transcript-actions" onClick={(event) => event.stopPropagation()}>
                <SmartOverflow
                  ariaLabel={`Actions for the transcript from ${formatTimestamp(entry.timestamp)}`}
                  moreLabel="More transcript actions"
                  actionClassName="transcript-action"
                  moreButtonClassName="transcript-more"
                  menuClassName="transcript-overflow-menu"
                >
                  <SmartOverflowAction
                    id="copy"
                    priority="primary"
                    icon={<Copy />}
                    onSelect={() => void handleCopy(entry)}
                  >
                    Copy
                  </SmartOverflowAction>
                  {isNewest && (
                    <SmartOverflowAction
                      id="teach"
                      priority="overflow"
                      icon={<GraduationCap />}
                      onSelect={() => setTeachingEntry(entry)}
                    >
                      Correct &amp; Teach
                    </SmartOverflowAction>
                  )}
                </SmartOverflow>
              </div>
              {entry.derived && <span className="transcript-derived">Reformatted · {entry.derived.modeId}</span>}
              </article>
            </Fragment>
          );
        })}
        {rendered.length < visible.length && (
          <button
            type="button"
            onClick={() => setRenderLimit((limit) => limit + HISTORY_RENDER_BATCH)}
            className="mx-auto my-3 rounded-lg bg-surface-container px-3 py-2 text-xs font-semibold text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          >
            Show {Math.min(HISTORY_RENDER_BATCH, visible.length - rendered.length)} older
          </button>
        )}
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

export const HistoryPanel = memo(HistoryPanelComponent);
