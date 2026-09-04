/**
 * Redesign variant A — "Adopt the primitives".
 *
 * Same information architecture as HomeDashboard (heading, recording bar,
 * styles tip, recent dictations with search + source filter, right rail with
 * This month / This week / Voice profile). The only thing that changes is
 * *what the controls are made of*: the four hand-rolled controls in the
 * current page are replaced with the Sona registry components, so the page
 * is shippable next week without re-teaching anyone the layout.
 */
import { Fragment, useEffect, useMemo, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'motion/react';
import {
  ClipboardCopy,
  Copy,
  Download,
  GraduationCap,
  Trash2,
} from 'lucide-react';

import BubbleUpButton from '@/components/ui/bubble-up-button/bubble-up-button';
import FluidTabs from '@/components/ui/fluid-tabs/fluid-tabs';
import SmartOverflow, { SmartOverflowAction } from '@/components/ui/smart-overflow/smart-overflow';
import {
  AnimatedDropdown,
  AnimatedDropdownContent,
  AnimatedDropdownItem,
  AnimatedDropdownSeparator,
  AnimatedDropdownTrigger,
} from '@/components/ui/animated-dropdown/animated-dropdown';

import {
  DashboardAction,
  DashboardSectionHeader,
  DashboardStatGroup,
  DashboardSurface,
} from '../../ui/DashboardPrimitives';
import { DayChart } from '../../ui/DayChart';
import { PersonalizationCard } from '../PersonalizationCard';
import {
  HISTORY_FILTER_OPTIONS,
  entrySource,
  filterHistory,
  formatTimestamp,
  matchSegments,
  sortForDisplay,
  type HistoryEntry,
  type HistoryFilter,
} from '../../../lib/history';
import { copyHistoryExport, saveHistoryExport } from '../../../lib/historyExport';
import { derivePersonalization, getUsageOverview } from '../../../lib/homeDashboard';
import { getRecentDays, loadStats } from '../../../lib/stats';
import { flog } from '../../../lib/log';
import type { DoubleTapKey, RecordingMode } from '../../../lib/settings';
import type { HomeRedesignProps } from './types';

import './variant-a.css';

const TIP_DISMISSED_KEY = 'murmur-home-styles-tip-dismissed';

/* ── Demo data ──────────────────────────────────────────────────────────────
 * DEMO ONLY. The visual fixture passes three short entries, which is not
 * enough surface to judge grouping, the source filter, the "Show more"
 * clamp, or per-entry actions. These six entries render instead whenever the
 * caller supplies fewer than five. Every timestamp is a fixed local date
 * anchored to 2026-09-04 so screenshots are byte-stable. */
const DEMO_NOW = new Date(2026, 8, 4, 16, 30).getTime();

function demoAt(day: number, hour: number, minute: number): number {
  return new Date(2026, 8, day, hour, minute).getTime();
}

const demoEntries: HistoryEntry[] = [
  {
    id: 'demo-1',
    text: 'Ship the redesign behind the fixture flag first, then promote whichever variant survives the 880-wide window without collapsing the rail.',
    timestamp: demoAt(4, 15, 42),
    duration: 9,
    source: 'recording',
  },
  {
    id: 'demo-2',
    text: 'Reminder for the release notes: transcription still runs entirely on this Mac. The Core ML path handles the ANE, whisper.cpp covers Metal, and sherpa-onnx is the CPU fallback. Nothing leaves the device, no account is required, and the clipboard is always written before auto-paste is attempted so a failed paste never loses the text.',
    timestamp: demoAt(4, 14, 8),
    duration: 41,
    source: 'recording',
  },
  {
    id: 'demo-3',
    text: 'Weekly sync: agreed to keep the benchmark corpus on the trusted Mac and publish only the content-free metric summary.',
    timestamp: demoAt(4, 11, 20),
    duration: 27,
    source: 'file',
    sourceName: 'weekly-sync.wav',
  },
  {
    id: 'demo-4',
    text: 'Draft the changelog entry for the capture supervisor: single owner, cancellation, and confirmed-termination evidence.',
    timestamp: demoAt(3, 18, 3),
    duration: 14,
    source: 'recording',
  },
  {
    id: 'demo-5',
    text: 'Interview notes — the reviewer wanted the export menu to stop feeling like a nested settings page.',
    timestamp: demoAt(3, 16, 45),
    duration: 33,
    source: 'file',
    sourceName: 'interview-04.m4a',
  },
  {
    id: 'demo-6',
    text: 'Check the notch geometry on the external display before tagging.',
    timestamp: demoAt(3, 9, 12),
    duration: 5,
    source: 'recording',
  },
];

/* ── Small helpers copied from the shipping page ────────────────────────── */

const KEY_LABELS: Record<DoubleTapKey, string> = {
  shift_l: '⇧ Shift',
  alt_l: '⌥ Option',
  ctrl_r: '⌃ Control',
};

function hotkeyHint(mode: RecordingMode, key: DoubleTapKey): string {
  if (mode === 'double_tap') return `Double-tap ${KEY_LABELS[key]} anywhere to begin`;
  if (mode === 'both') return `Hold or double-tap ${KEY_LABELS[key]} anywhere to begin`;
  return `Hold ${KEY_LABELS[key]} anywhere to begin`;
}

function timer(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, '0')}`;
}

function dayKeyOf(timestamp: number): string {
  const date = new Date(timestamp);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

/** `reference` is passed explicitly so demo data never depends on the clock. */
function dayLabelOf(timestamp: number, reference: number): string {
  const today = new Date(reference);
  const yesterday = new Date(reference);
  yesterday.setDate(today.getDate() - 1);
  const key = dayKeyOf(timestamp);
  if (key === dayKeyOf(today.getTime())) return 'Today';
  if (key === dayKeyOf(yesterday.getTime())) return 'Yesterday';
  return new Date(timestamp).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  });
}

function loadTipDismissed(): boolean {
  try { return localStorage.getItem(TIP_DISMISSED_KEY) === 'true'; }
  catch { return false; }
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

/** Two-line clamp with the same "Show more" affordance the current app has. */
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
          className="variant-a-showmore focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          onClick={(event) => {
            event.stopPropagation();
            setExpanded((value) => !value);
          }}
        >
          {expanded ? 'Show less' : 'Show more'}
        </button>
      )}
    </>
  );
}

/* ── Variant ────────────────────────────────────────────────────────────── */

export function VariantA({
  historyEntries,
  onClearHistory,
  onUpdateHistoryEntry,
  onTranscribeFile,
  status,
  initialized,
  recordingDuration,
  audioLevel,
  settings,
  meetings,
  statsVersion,
  onRecord,
  onStop,
  onOpenInsights,
  onOpenSettings,
}: HomeRedesignProps) {
  const [tipDismissed, setTipDismissed] = useState(loadTipDismissed);
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<HistoryFilter>('all');
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const usingDemoData = historyEntries.length < 5;
  const entries = usingDemoData ? demoEntries : historyEntries;
  const reference = usingDemoData ? DEMO_NOW : new Date().getTime();

  const stats = useMemo(() => loadStats(), [statsVersion]);
  const usage = useMemo(() => getUsageOverview(stats), [stats]);
  const personalization = useMemo(
    () => derivePersonalization(settings, stats),
    [settings.vocabularyEntries, settings.appProfiles, stats],
  );
  const recent = useMemo(() => getRecentDays(stats, 7), [stats]);

  const hasConfiguredStyle = settings.appProfiles.some((profile) => profile.writingStyle !== null);
  const showStylesTip = !tipDismissed && !hasConfiguredStyle;

  useEffect(() => {
    if (hasConfiguredStyle) setTipDismissed(true);
  }, [hasConfiguredStyle]);

  const visible = useMemo(
    () => sortForDisplay(filterHistory(entries, { query, filter })),
    [entries, query, filter],
  );

  const isCapturing = status === 'starting' || status === 'recording';
  const busy = status === 'processing' || status === 'recovering';
  const meetingBusy = meetings.status.phase !== 'idle' && meetings.status.phase !== 'failed';
  const normalized = Math.min(1, Math.max(0, audioLevel) * 16);
  const envelopes = [0.52, 0.78, 1, 0.78, 0.52];

  const statusTitle = status === 'starting'
    ? 'Connecting to microphone'
    : status === 'recording'
      ? `Recording · ${timer(recordingDuration)}`
      : status === 'processing'
        ? 'Processing locally'
        : status === 'recovering'
          ? 'Recovering microphone'
          : initialized
            ? 'Ready to dictate'
            : 'Initializing';
  const actionLabel = status === 'starting'
    ? 'Cancel'
    : status === 'recording'
      ? 'Stop Recording'
      : status === 'processing'
        ? 'Processing'
        : status === 'recovering'
          ? 'Recovering'
          : 'Start Recording';

  const dismissTip = () => {
    setTipDismissed(true);
    try { localStorage.setItem(TIP_DISMISSED_KEY, 'true'); } catch { /* presentation-only */ }
  };

  const copyEntry = async (entry: HistoryEntry) => {
    try {
      await navigator.clipboard.writeText(entry.text);
      setCopiedId(entry.id);
    } catch (err) {
      setNotice('Could not copy to the clipboard.');
      flog.warn('main', 'Variant A copy failed', { error: String(err) });
    }
  };

  const copyAll = async () => {
    try {
      const count = await copyHistoryExport(visible, 'text');
      setNotice(`Copied ${count} ${count === 1 ? 'entry' : 'entries'} to the clipboard.`);
    } catch (err) {
      setNotice('Export to clipboard failed.');
      flog.warn('main', 'Variant A export copy failed', { error: String(err) });
    }
  };

  const saveAll = async () => {
    try {
      const path = await saveHistoryExport(visible, 'text');
      if (path) setNotice(`Saved ${visible.length} ${visible.length === 1 ? 'entry' : 'entries'}.`);
    } catch (err) {
      setNotice('Could not save the export.');
      flog.warn('main', 'Variant A export save failed', { error: String(err) });
    }
  };

  return (
    <div className="home-dashboard" data-redesign-variant="A">
      <header className="home-dashboard-heading">
        <div>
          <h1>Ready when you are</h1>
          <p>
            {usage.recordingsThisMonth.toLocaleString()}{' '}
            {usage.recordingsThisMonth === 1 ? 'dictation' : 'dictations'} this month · everything processed locally
          </p>
        </div>
      </header>

      <div className="home-dashboard-grid">
        <div className="home-dashboard-main">
          <DashboardSurface as="section" variant="outlined" padding="compact" ariaLabel="Dictation controls">
            <div className="home-recording-bar">
              {/* Sona BubbleUpButton: the fill rises from the bottom edge on
                  hover/focus, and the status label swaps with a contained
                  upward slide so Start → Stop → Processing reads as one
                  control changing state rather than three buttons. */}
              <BubbleUpButton
                className="variant-a-record"
                data-testid="home-record-button"
                data-tone={isCapturing ? 'danger' : 'default'}
                disabled={!initialized || busy || meetingBusy}
                onClick={() => void (isCapturing ? onStop() : onRecord())}
                aria-label={
                  status === 'recording'
                    ? `Stop recording, ${timer(recordingDuration)}`
                    : status === 'starting'
                      ? 'Cancel recording'
                      : busy
                        ? statusTitle
                        : 'Start recording'
                }
              >
                <span className="home-record-dot" aria-hidden="true" />
                <span className="variant-a-record-label">
                  <AnimatePresence initial={false} mode="popLayout">
                    <motion.span
                      key={actionLabel}
                      initial={{ y: '115%', opacity: 0 }}
                      animate={{ y: '0%', opacity: 1 }}
                      exit={{ y: '-115%', opacity: 0 }}
                      transition={{ type: 'spring', stiffness: 420, damping: 38 }}
                    >
                      {actionLabel}
                    </motion.span>
                  </AnimatePresence>
                </span>
              </BubbleUpButton>

              <div className="home-record-state" aria-live="polite">
                <div className="home-record-state-line">
                  <strong>{statusTitle}</strong>
                  {status === 'recording' && (
                    <span className="home-record-waveform" aria-hidden="true">
                      {envelopes.map((envelope, index) => (
                        <span key={index} style={{ height: `${Math.max(3, Math.round((0.15 + normalized * envelope) * 18))}px` }} />
                      ))}
                    </span>
                  )}
                </div>
                <span className="home-record-hint">{hotkeyHint(settings.recordingMode, settings.doubleTapKey)}</span>
              </div>

              <DashboardAction
                variant="secondary"
                onActivate={onTranscribeFile}
                disabled={isCapturing || busy || meetingBusy}
              >
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.7} aria-hidden="true">
                  <path strokeLinecap="round" strokeLinejoin="round" d="M7 3h7l4 4v14H7V3Z" />
                  <path strokeLinecap="round" strokeLinejoin="round" d="M14 3v5h5M12 17V11m0 0-3 3m3-3 3 3" />
                </svg>
                Transcribe File
              </DashboardAction>
            </div>
          </DashboardSurface>

          {showStylesTip && (
            <section className="home-styles-tip" aria-label="Set up app styles">
              <span className="home-tip-icon" aria-hidden="true">⌁</span>
              <p><strong>Make Murmur sound like you.</strong> Set a writing style for each app.</p>
              <button type="button" onClick={() => onOpenSettings({ page: 'delivery', target: 'app-overrides' })}>Set up styles</button>
              <button type="button" onClick={dismissTip} aria-label="Dismiss styles tip" className="home-tip-dismiss">×</button>
            </section>
          )}

          <section className="home-history" aria-labelledby="variant-a-recent-title">
            <div className="home-history-heading">
              <h2 id="variant-a-recent-title">Recent dictations</h2>
              <span>{entries.length} {entries.length === 1 ? 'entry' : 'entries'}</span>
            </div>

            <div className="variant-a-toolbar">
              <div className="history-search" data-expanded="true">
                <span className="pointer-events-none absolute inset-y-0 left-0 z-10 grid w-7 place-items-center text-on-surface-variant">
                  <svg className="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-4.35-4.35M17 11a6 6 0 11-12 0 6 6 0 0112 0z" />
                  </svg>
                </span>
                <input
                  type="search"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  onKeyDown={(event) => { if (event.key === 'Escape') { event.preventDefault(); setQuery(''); } }}
                  placeholder="Search transcripts"
                  aria-label="Search transcripts"
                  className="absolute inset-0 h-full w-full border-0 bg-transparent py-1 pl-7 pr-7 text-sm text-on-surface outline-none placeholder:text-on-surface-variant [&::-webkit-search-cancel-button]:appearance-none"
                />
                {query ? (
                  <button
                    type="button"
                    onClick={() => setQuery('')}
                    aria-label="Clear transcript search"
                    className="absolute right-1 top-1 z-10 grid h-5 w-5 place-items-center rounded bg-on-surface/5 text-xs leading-none text-on-surface-variant transition-colors hover:bg-on-surface/10 hover:text-on-surface focus:outline-none focus-visible:ring-1 focus-visible:ring-primary"
                  >
                    ×
                  </button>
                ) : (
                  <kbd className="pointer-events-none absolute right-2 top-1 rounded-full bg-on-surface/6 px-1 py-0.5 font-mono text-[9px] text-on-surface-variant">/</kbd>
                )}
              </div>

              {/* Sona FluidTabs, capsule variant: the selected chip is one
                  shared indicator that glides between All / Mic / File. */}
              <FluidTabs
                tabs={HISTORY_FILTER_OPTIONS.map((option) => ({ value: option.value, title: option.label }))}
                value={filter}
                onValueChange={(next) => setFilter(next as HistoryFilter)}
                variant="capsule"
                size="sm"
                ariaLabel="Filter transcripts"
                className="variant-a-tabs"
                listClassName="variant-a-tabs-list"
                activeIndicatorClassName="variant-a-tab-indicator"
              />

              <span className="variant-a-count">
                {visible.length === entries.length
                  ? `${entries.length} ${entries.length === 1 ? 'entry' : 'entries'}`
                  : `${visible.length} of ${entries.length}`}
              </span>

              {/* Sona AnimatedDropdown replaces the hand-rolled export popover
                  and its outside-click/Escape bookkeeping. */}
              <AnimatedDropdown>
                <AnimatedDropdownTrigger className="variant-a-export-trigger">
                  <span aria-hidden="true" className="text-base leading-none">···</span>
                  <span className="sr-only">More history actions</span>
                </AnimatedDropdownTrigger>
                <AnimatedDropdownContent align="end" side="bottom" className="variant-a-menu min-w-52">
                  <AnimatedDropdownItem icon={<ClipboardCopy />} onClick={() => void copyAll()}>
                    Copy all
                  </AnimatedDropdownItem>
                  <AnimatedDropdownItem icon={<Download />} onClick={() => void saveAll()}>
                    Save as text…
                  </AnimatedDropdownItem>
                  <AnimatedDropdownSeparator />
                  <AnimatedDropdownItem
                    variant="danger"
                    className="variant-a-danger-item"
                    icon={<Trash2 />}
                    onClick={onClearHistory}
                  >
                    Clear history
                  </AnimatedDropdownItem>
                </AnimatedDropdownContent>
              </AnimatedDropdown>
            </div>

            {notice && (
              <p role="status" className="mb-1.5 rounded-lg bg-surface-container px-2.5 py-1.5 text-[11px] text-on-surface-variant">{notice}</p>
            )}

            <div className="variant-a-history-list">
              {visible.length === 0 ? (
                <div className="variant-a-empty">
                  <span className="grid h-12 w-12 place-items-center rounded-2xl bg-surface-container-low text-xl" aria-hidden="true">◌</span>
                  <p className="font-medium text-on-surface">No matching transcripts</p>
                  <button
                    type="button"
                    className="variant-a-showmore"
                    onClick={() => { setQuery(''); setFilter('all'); }}
                  >
                    Reset filters
                  </button>
                </div>
              ) : visible.map((entry, index) => {
                const showDayLabel = index === 0 || dayKeyOf(entry.timestamp) !== dayKeyOf(visible[index - 1].timestamp);
                const endsDay = index === visible.length - 1 || dayKeyOf(entry.timestamp) !== dayKeyOf(visible[index + 1].timestamp);
                return (
                  <Fragment key={entry.id}>
                    {showDayLabel && <p className="history-date-label">{dayLabelOf(entry.timestamp, reference)}</p>}
                    <article
                      data-testid="transcript-card"
                      data-copied={copiedId === entry.id}
                      data-day-end={endsDay}
                      data-newest={index === 0}
                      className="transcript-card group"
                      aria-label={`Transcription from ${formatTimestamp(entry.timestamp)}`}
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
                        </div>
                      </div>

                      <ClampedTranscript text={entry.text} query={query} />

                      <span className="transcript-copy-feedback" role="status" aria-live="polite">
                        {copiedId === entry.id ? 'Copied' : ''}
                      </span>

                      {/* Sona SmartOverflow: Copy stays visible longest,
                          Correct & Teach folds next, Delete always lives in
                          the destructive overflow menu. */}
                      <SmartOverflow
                        className="variant-a-actions"
                        ariaLabel={`Actions for the transcript from ${formatTimestamp(entry.timestamp)}`}
                        moreLabel="More transcript actions"
                        actionClassName="variant-a-action"
                        moreButtonClassName="variant-a-more"
                        menuClassName="variant-a-overflow-menu"
                      >
                        <SmartOverflowAction
                          id="copy"
                          priority="primary"
                          icon={<Copy />}
                          onSelect={() => void copyEntry(entry)}
                        >
                          Copy
                        </SmartOverflowAction>
                        <SmartOverflowAction
                          id="teach"
                          priority="secondary"
                          icon={<GraduationCap />}
                          onSelect={() => onUpdateHistoryEntry(entry.id, entry.text)}
                        >
                          Correct &amp; Teach
                        </SmartOverflowAction>
                        <SmartOverflowAction
                          id="delete"
                          priority="overflow"
                          destructive
                          icon={<Trash2 />}
                          onSelect={onClearHistory}
                        >
                          Delete
                        </SmartOverflowAction>
                      </SmartOverflow>
                    </article>
                  </Fragment>
                );
              })}
            </div>
          </section>
        </div>

        <aside className="home-insights-rail" aria-label="Usage summary">
          <DashboardSurface as="section" variant="outlined" padding="standard">
            <DashboardSectionHeader
              eyebrow="This month"
              action={(
                <DashboardAction variant="quiet" icon="forward" onActivate={onOpenInsights}>
                  View insights
                </DashboardAction>
              )}
            />
            <DashboardStatGroup
              kind="rows"
              ariaLabel="Usage this month"
              items={[
                { id: 'words', label: 'Words', value: usage.wordsThisMonth.toLocaleString() },
                { id: 'wpm', label: 'Average WPM', value: usage.averageWpm || '—' },
                { id: 'recordings', label: 'Recordings', value: usage.recordingsThisMonth.toLocaleString() },
                { id: 'streak', label: 'Day streak', value: usage.currentStreak },
              ]}
            />
          </DashboardSurface>

          <DashboardSurface as="section" variant="outlined" padding="standard">
            <DashboardSectionHeader eyebrow="This week" />
            <DayChart
              kind="bars"
              metric="words"
              days={recent}
              density="compact"
              highlightLast
              ariaLabel="Words per day for the last seven days"
            />
          </DashboardSurface>

          <PersonalizationCard
            summary={personalization}
            onOpenVocabulary={() => onOpenSettings({ page: 'text', editorTab: 'aliases' })}
            onOpenStyles={() => onOpenSettings({ page: 'delivery', target: 'app-overrides' })}
          />
        </aside>
      </div>
    </div>
  );
}
