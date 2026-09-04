import { Fragment, useId, useMemo, useState } from 'react';

import ActivityGraph, {
  type ActivityGraphDatum,
  type ActivityGraphValueContext,
} from '@/components/ui/activity-graph/activity-graph';
import ExpandingAction from '@/components/ui/expanding-action/expanding-action';
import FluidTabs from '@/components/ui/fluid-tabs/fluid-tabs';
import SpotlightCard from '@/components/ui/spotlight-card/spotlight-card';

import {
  DashboardAction,
  DashboardSurface,
} from '../../ui/DashboardPrimitives';
import { PersonalizationCard } from '../PersonalizationCard';
import {
  entrySource,
  filterHistory,
  formatTimestamp,
  matchSegments,
  sortForDisplay,
  HISTORY_FILTER_OPTIONS,
  type HistoryEntry,
  type HistoryFilter,
} from '../../../lib/history';
import { derivePersonalization, getUsageOverview } from '../../../lib/homeDashboard';
import { loadStats } from '../../../lib/stats';
import type { DoubleTapKey, RecordingMode } from '../../../lib/settings';
import type { HomeRedesignProps } from './types';

import './variant-b.css';

/* ------------------------------------------------------------------ *
 * Demo data — DESIGN FIXTURE ONLY.
 *
 * The visual fixture seeds seven days of stats and three transcripts,
 * which is far too thin to judge a rhythm-led layout. Everything below
 * is deterministic (seeded mulberry32, no Math.random, no Date.now) and
 * is only used when the real history is thinner than five entries. It
 * is anchored to 2026-09-04 so screenshots never drift.
 * ------------------------------------------------------------------ */

/** Last day covered by the demo activity series (a Friday). */
const DEMO_END = { year: 2026, month: 8, day: 4 } as const;
/** First day of the demo window: the Sunday exactly 16 columns earlier. */
const DEMO_START_ISO = '2026-05-17';
const DEMO_END_ISO = '2026-09-04';
const DEMO_WEEKS = 16;
/** 2026-05-17 → 2026-09-04 inclusive. */
const DEMO_DAY_COUNT = 111;
/** Recent days are always active so the streak tile reads as a real habit. */
const DEMO_ACTIVE_TAIL = 6;

function mulberry32(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

interface DemoDay {
  iso: string;
  words: number;
  dictations: number;
  weekday: number;
}

function buildDemoSeries(): DemoDay[] {
  const random = mulberry32(0x4d55524d); // "MURM"
  const end = Date.UTC(DEMO_END.year, DEMO_END.month, DEMO_END.day);
  const days: DemoDay[] = [];

  for (let offset = DEMO_DAY_COUNT - 1; offset >= 0; offset -= 1) {
    const date = new Date(end - offset * 86_400_000);
    const weekday = date.getUTCDay();
    const weekend = weekday === 0 || weekday === 6;
    const inTail = offset < DEMO_ACTIVE_TAIL;
    const roll = random();
    const skip = !inTail && roll < (weekend ? 0.44 : 0.1);
    // Weekends stay deliberately lighter than weekdays.
    const base = weekend ? 150 : 620;
    const spread = weekend ? 240 : 700;
    const words = skip ? 0 : Math.round(base + random() * spread);
    days.push({
      iso: date.toISOString().slice(0, 10),
      words,
      dictations: words === 0 ? 0 : Math.max(1, Math.round(words / 148)),
      weekday,
    });
  }

  return days;
}

const DEMO_SERIES = buildDemoSeries();

const DEMO_GRAPH_DATA: ActivityGraphDatum[] = DEMO_SERIES.map((day) => ({
  date: day.iso,
  value: day.words,
  label: `${day.words.toLocaleString()} words`,
  metadata: { dictations: day.dictations },
}));

const DEMO_TOTAL_DICTATIONS = DEMO_SERIES.reduce((sum, day) => sum + day.dictations, 0);
const DEMO_ACTIVE_DAYS = DEMO_SERIES.filter((day) => day.words > 0).length;
const DEMO_BEST_DAY = DEMO_SERIES.reduce(
  (best, day) => (day.words > best ? day.words : best),
  0,
);

function demoStreak(): number {
  let streak = 0;
  for (let index = DEMO_SERIES.length - 1; index >= 0; index -= 1) {
    if (DEMO_SERIES[index].words === 0) break;
    streak += 1;
  }
  return streak;
}

const DEMO_MONTH_PREFIX = '2026-09-';
const DEMO_MONTH = DEMO_SERIES.filter((day) => day.iso.startsWith(DEMO_MONTH_PREFIX));
const DEMO_USAGE = {
  wordsThisMonth: DEMO_MONTH.reduce((sum, day) => sum + day.words, 0),
  recordingsThisMonth: DEMO_MONTH.reduce((sum, day) => sum + day.dictations, 0),
  averageWpm: 187,
  currentStreak: demoStreak(),
};

/** Local-clock timestamps, so the Today/Yesterday grouping stays correct. */
function demoAt(day: number, hour: number, minute: number): number {
  return new Date(DEMO_END.year, DEMO_END.month, day, hour, minute).getTime();
}

const DEMO_ENTRIES: HistoryEntry[] = [
  {
    id: 'demo-1',
    text: 'Ship the rhythm rail behind the redesign flag, then measure how often the graph is the thing people actually look at.',
    timestamp: demoAt(4, 15, 42),
    duration: 11,
    source: 'recording',
  },
  {
    id: 'demo-2',
    text: 'Reminder for the standup: the sidecar handshake retry is merged, the capture probe is not, and the notch geometry fixture still needs a second pass.',
    timestamp: demoAt(4, 13, 8),
    duration: 17,
    source: 'recording',
  },
  {
    id: 'demo-3',
    text: 'Transcript of the weekly design review covering the activity graph density, the stat tile hierarchy, and the expanding start control.',
    timestamp: demoAt(4, 11, 25),
    duration: 214,
    source: 'file',
    sourceName: 'design-review.wav',
  },
  {
    id: 'demo-4',
    text: 'Draft reply: yes to the earlier slot, no to moving the release, and I will send the benchmark summary before Friday.',
    timestamp: demoAt(4, 9, 3),
    duration: 9,
    source: 'recording',
  },
  {
    id: 'demo-5',
    text: 'Everything in this window is processed on this Mac, which is the part of the pitch that keeps surviving every rewrite.',
    timestamp: demoAt(3, 17, 51),
    duration: 8,
    source: 'recording',
  },
  {
    id: 'demo-6',
    text: 'Imported voice memo about the onboarding permissions wizard and the model download retry copy.',
    timestamp: demoAt(3, 16, 12),
    duration: 96,
    source: 'file',
    sourceName: 'voice-memo-041.m4a',
  },
];

/* ------------------------------------------------------------------ */

const KEY_CHIPS: Record<DoubleTapKey, readonly [string, string]> = {
  shift_l: ['⇧', 'Shift'],
  alt_l: ['⌥', 'Option'],
  ctrl_r: ['⌃', 'Control'],
};

function hotkeyVerb(mode: RecordingMode): string {
  if (mode === 'double_tap') return 'Double-tap';
  if (mode === 'both') return 'Hold or double-tap';
  return 'Hold';
}

function timer(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, '0')}`;
}

function dayKeyOf(timestamp: number): string {
  const date = new Date(timestamp);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

function dayLabelOf(timestamp: number): string {
  const date = new Date(timestamp);
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  const key = dayKeyOf(timestamp);
  if (key === dayKeyOf(today.getTime())) return 'Today';
  if (key === dayKeyOf(yesterday.getTime())) return 'Yesterday';
  return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
}

function Highlighted({ text, query }: { text: string; query: string }) {
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

function MicIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.9} className="h-full w-full" aria-hidden="true">
      <path strokeLinecap="round" strokeLinejoin="round" d="M19 11a7 7 0 0 1-14 0m7 7v3m-4 0h8m-4-6a3 3 0 0 1-3-3V5a3 3 0 0 1 6 0v4a3 3 0 0 1-3 3Z" />
    </svg>
  );
}

const GRAPH_LEVEL_COLORS = [
  'color-mix(in oklab, var(--murmur-primary) 22%, var(--murmur-background))',
  'color-mix(in oklab, var(--murmur-primary) 45%, var(--murmur-background))',
  'color-mix(in oklab, var(--murmur-primary) 70%, var(--murmur-background))',
  'var(--murmur-primary)',
];

export function VariantB({
  historyEntries,
  onTranscribeFile,
  status,
  initialized,
  recordingDuration,
  settings,
  meetings,
  statsVersion,
  onRecord,
  onStop,
  onOpenInsights,
  onOpenSettings,
}: HomeRedesignProps) {
  const [filter, setFilter] = useState<HistoryFilter>('all');
  const [query, setQuery] = useState('');
  const [startOpen, setStartOpen] = useState(false);
  const listId = useId();
  const searchId = useId();

  const stats = useMemo(() => loadStats(), [statsVersion]);
  const realUsage = useMemo(() => getUsageOverview(stats), [stats]);
  const personalization = useMemo(
    () => derivePersonalization(settings, stats),
    [settings.vocabularyEntries, settings.appProfiles, stats],
  );

  // Demo mode only kicks in when the real history is too thin to lay out.
  const demo = historyEntries.length < 5;
  const entries = demo ? DEMO_ENTRIES : historyEntries;
  const usage = demo ? DEMO_USAGE : realUsage;

  const graphData = useMemo<ActivityGraphDatum[]>(() => {
    if (demo) return DEMO_GRAPH_DATA;
    return Object.entries(stats.dailyBuckets)
      .filter(([, bucket]) => bucket.words > 0)
      .map(([key, bucket]) => ({
        date: key,
        value: bucket.words,
        label: `${bucket.words.toLocaleString()} words`,
        metadata: { dictations: bucket.recordings },
      }));
  }, [demo, stats]);

  const graphSummary = demo
    ? `${DEMO_WEEKS} weeks · ${DEMO_TOTAL_DICTATIONS.toLocaleString()} dictations`
    : `${DEMO_WEEKS} weeks · ${realUsage.totalRecordings.toLocaleString()} dictations`;
  const graphDetail = demo
    ? `${DEMO_ACTIVE_DAYS} active days · best ${DEMO_BEST_DAY.toLocaleString()} words`
    : `${realUsage.activeDaysThisMonth} active days this month`;

  const visible = useMemo(
    () => sortForDisplay(filterHistory(entries, { query, filter })),
    [entries, query, filter],
  );

  const isCapturing = status === 'starting' || status === 'recording';
  const busy = status === 'processing' || status === 'recovering';
  const meetingBusy = meetings.status.phase !== 'idle' && meetings.status.phase !== 'failed';
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
  const statusTone = isCapturing ? 'live' : busy ? 'busy' : initialized ? 'ready' : 'idle';
  const [keySymbol, keyName] = KEY_CHIPS[settings.doubleTapKey];

  const tiles = [
    { id: 'words', label: 'Words', value: usage.wordsThisMonth.toLocaleString(), detail: 'this month' },
    { id: 'wpm', label: 'Average WPM', value: usage.averageWpm || '—', detail: 'recent sessions' },
    { id: 'recordings', label: 'Recordings', value: usage.recordingsThisMonth.toLocaleString(), detail: 'this month' },
    { id: 'streak', label: 'Day streak', value: usage.currentStreak, detail: 'consecutive days' },
  ];

  return (
    <div className="home-dashboard vb-root" data-redesign-variant="B">
      <header className="home-dashboard-heading vb-heading">
        <div>
          <h1>Ready when you are</h1>
          <p>
            {usage.recordingsThisMonth.toLocaleString()}{' '}
            {usage.recordingsThisMonth === 1 ? 'dictation' : 'dictations'} this month · everything processed locally
          </p>
        </div>
      </header>

      <div className="home-dashboard-grid vb-grid">
        <div className="home-dashboard-main vb-main">
          <DashboardSurface as="section" variant="outlined" padding="compact" ariaLabel="Dictation controls">
            <div className="vb-hero">
              {isCapturing ? (
                <DashboardAction
                  testId="home-record-button"
                  variant="primary"
                  tone="danger"
                  onActivate={onStop}
                  ariaLabel={status === 'recording' ? `Stop recording, ${timer(recordingDuration)}` : 'Cancel recording'}
                >
                  <span className="home-record-dot" aria-hidden="true" />
                  <span>{status === 'starting' ? 'Cancel' : 'Stop'}</span>
                </DashboardAction>
              ) : (
                <ExpandingAction
                  className="vb-expanding"
                  triggerClassName="vb-expanding-trigger"
                  optionClassName="vb-expanding-option"
                  backLabel="Close start options"
                  open={startOpen}
                  onOpenChange={setStartOpen}
                  disabled={!initialized || busy || meetingBusy}
                  trigger="Start"
                  triggerIcon={<MicIcon />}
                  items={[
                    { value: 'record', label: 'Record' },
                    { value: 'file', label: 'Transcribe file' },
                    { value: 'meeting', label: 'Meeting' },
                  ]}
                  onValueSelect={(value) => {
                    if (value === 'record') onRecord();
                    else if (value === 'file') onTranscribeFile();
                    else void meetings.start();
                  }}
                />
              )}

              <div className="vb-hero-state" aria-live="polite">
                <p className="vb-hero-status" data-tone={statusTone}>
                  <span className="vb-hero-dot" aria-hidden="true" />
                  <strong>{statusTitle}</strong>
                </p>
                <p className="vb-hero-hint">
                  <span>{hotkeyVerb(settings.recordingMode)}</span>
                  <kbd>{keySymbol}</kbd>
                  <kbd>{keyName}</kbd>
                  <span>anywhere</span>
                </p>
              </div>
            </div>
          </DashboardSurface>

          <section className="home-history vb-history" aria-labelledby="vb-recent-title">
            <div className="home-history-heading vb-history-heading">
              <h2 id="vb-recent-title">Recent dictations</h2>
              <span>
                {visible.length === entries.length
                  ? `${entries.length} ${entries.length === 1 ? 'entry' : 'entries'}`
                  : `${visible.length} of ${entries.length}`}
              </span>
            </div>

            <div className="vb-history-toolbar">
              <FluidTabs
                className="vb-tabs"
                listClassName="vb-tabs-list"
                size="sm"
                ariaLabel="Filter transcripts"
                value={filter}
                onValueChange={(value) => setFilter(value as HistoryFilter)}
                tabs={HISTORY_FILTER_OPTIONS.map((option) => ({
                  value: option.value,
                  title: option.label,
                  ariaControls: listId,
                }))}
              />

              <div className="history-search vb-search" data-expanded="true">
                <span className="pointer-events-none absolute inset-y-0 left-0 z-10 grid w-7 place-items-center text-on-surface-variant">
                  <svg className="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-4.35-4.35M17 11a6 6 0 11-12 0 6 6 0 0112 0z" />
                  </svg>
                </span>
                <input
                  id={searchId}
                  type="search"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key !== 'Escape') return;
                    event.preventDefault();
                    event.stopPropagation();
                    setQuery('');
                  }}
                  placeholder="Search transcripts"
                  aria-label="Search transcripts"
                  className="absolute inset-0 h-full w-full border-0 bg-transparent py-1 pl-7 pr-7 text-sm text-on-surface outline-none placeholder:text-on-surface-variant [&::-webkit-search-cancel-button]:appearance-none"
                />
                {query && (
                  <button
                    type="button"
                    onClick={() => setQuery('')}
                    aria-label="Clear transcript search"
                    className="absolute right-1 top-1 z-10 grid h-5 w-5 place-items-center rounded bg-on-surface/5 text-xs leading-none text-on-surface-variant hover:bg-on-surface/10 hover:text-on-surface focus:outline-none focus-visible:ring-1 focus-visible:ring-primary"
                  >
                    ×
                  </button>
                )}
              </div>
            </div>

            <div className="history-list vb-list" id={listId}>
              {visible.length === 0 ? (
                <div className="vb-empty">
                  <p>No matching transcripts</p>
                  <button type="button" onClick={() => { setQuery(''); setFilter('all'); }}>Reset filters</button>
                </div>
              ) : visible.map((entry, index) => {
                const showDayLabel = index === 0
                  || dayKeyOf(entry.timestamp) !== dayKeyOf(visible[index - 1].timestamp);
                const endsDay = index === visible.length - 1
                  || dayKeyOf(entry.timestamp) !== dayKeyOf(visible[index + 1].timestamp);
                return (
                  <Fragment key={entry.id}>
                    {showDayLabel && <p className="history-date-label">{dayLabelOf(entry.timestamp)}</p>}
                    <article
                      data-testid="transcript-card"
                      data-day-end={endsDay}
                      data-newest={index === 0}
                      className="transcript-card group"
                      role="group"
                      tabIndex={0}
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
                      <p className="transcript-text vb-transcript-text">
                        <Highlighted text={entry.text} query={query} />
                      </p>
                    </article>
                  </Fragment>
                );
              })}
            </div>
          </section>
        </div>

        <aside className="home-insights-rail vb-rail" aria-label="Usage rhythm">
          <DashboardSurface as="section" variant="outlined" padding="standard" ariaLabel="Dictation rhythm">
            <div className="vb-rail-head">
              <p className="ui-dashboard-eyebrow">Rhythm</p>
              <DashboardAction variant="quiet" icon="forward" onActivate={onOpenInsights}>Insights</DashboardAction>
            </div>

            <div className="vb-graph">
              <ActivityGraph
                data={graphData}
                startDate={DEMO_START_ISO}
                endDate={DEMO_END_ISO}
                maxDays={DEMO_WEEKS * 7}
                levels={4}
                weekStartsOn={0}
                colors={GRAPH_LEVEL_COLORS}
                emptyColor="color-mix(in oklab, var(--murmur-outline-variant) 28%, var(--murmur-surface-container-low))"
                showValue={false}
                showWeekdayLabels={false}
                showLegend
                showTooltip
                tooltipDelay={90}
                ariaLabel="Words dictated per day"
                emptyLabel="No dictation"
                cellClassName="vb-graph-cell"
                legendClassName="vb-graph-legend"
                renderTooltip={(context: ActivityGraphValueContext) => {
                  const meta = context.item?.metadata as { dictations?: number } | undefined;
                  return (
                    <div className="vb-graph-tooltip">
                      <strong>
                        {context.value === 0 ? 'No dictation' : `${context.value.toLocaleString()} words`}
                      </strong>
                      <span>
                        {context.date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', timeZone: 'UTC' })}
                        {meta?.dictations ? ` · ${meta.dictations} dictations` : ''}
                      </span>
                    </div>
                  );
                }}
                style={{
                  '--activity-graph-cell-size': 'var(--vb-cell-size)',
                  '--activity-graph-cell-gap': 'var(--vb-cell-gap)',
                  '--activity-graph-tooltip-surface': 'var(--murmur-surface-container-lowest)',
                  '--activity-graph-tooltip-foreground': 'var(--murmur-on-surface)',
                  '--activity-graph-muted-label': 'var(--murmur-on-surface-variant)',
                  '--activity-graph-label': 'var(--murmur-on-surface)',
                } as React.CSSProperties}
              />
            </div>

            <p className="vb-graph-summary">
              <strong>{graphSummary}</strong>
              <span>{graphDetail}</span>
            </p>
          </DashboardSurface>

          <div className="vb-tiles-block">
            <div className="vb-rail-head">
              <p className="ui-dashboard-eyebrow">This month</p>
              {demo && <span className="vb-demo-chip">Demo data</span>}
            </div>
            <div className="vb-tiles">
              {tiles.map((tile) => (
                <SpotlightCard
                  key={tile.id}
                  className="vb-tile rounded-[var(--ui-radius-card)] border-(--ui-hairline) bg-[var(--ui-tint-raised)] p-2.5 shadow-[var(--ui-shadow-1)]"
                  spotlightColor="color-mix(in srgb, var(--murmur-primary) 20%, transparent)"
                  spotlightSize={180}
                >
                  <p className="vb-tile-label">{tile.label}</p>
                  <p className="vb-tile-value">{tile.value}</p>
                  <p className="vb-tile-detail">{tile.detail}</p>
                </SpotlightCard>
              ))}
            </div>
          </div>

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
