import { useMemo, useState, type CSSProperties } from 'react';
import { Copy, GraduationCap, Trash2 } from 'lucide-react';

import ActivityGraph from '@/components/ui/activity-graph/activity-graph';
import FluidTabs from '@/components/ui/fluid-tabs/fluid-tabs';
import RippleButton, { RippleButtonText } from '@/components/ui/ripple-button/ripple-button';
import SmartOverflow, { SmartOverflowAction } from '@/components/ui/smart-overflow/smart-overflow';
import { DashboardAction, DashboardSurface } from '@/components/ui/DashboardPrimitives';

import {
  entrySource,
  filterHistory,
  formatTimestamp,
  sortForDisplay,
  type HistoryEntry,
  type HistoryFilter,
} from '../../../lib/history';
import { derivePersonalization, getUsageOverview } from '../../../lib/homeDashboard';
import { loadStats } from '../../../lib/stats';
import type { DoubleTapKey, RecordingMode } from '../../../lib/settings';
import type { HomeRedesignProps } from './types';
import './variant-c.css';

/* ------------------------------------------------------------------------ *
 * Direction C — "Quiet focus".
 *
 * One tab row replaces the section header and the sidebar-duplicated
 * destinations, the hero loses its card, and the whole right rail collapses
 * into a single outlined card. Everything below is presentation only.
 * ------------------------------------------------------------------------ */

const KEY_CHIPS: Record<DoubleTapKey, string[]> = {
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
  const wholeSeconds = Math.max(0, Math.floor(seconds));
  return `${Math.floor(wholeSeconds / 60)}:${String(wholeSeconds % 60).padStart(2, '0')}`;
}

/* --- Demo data ----------------------------------------------------------- *
 * DEMO ONLY. The visual fixture seeds 7 days of stats and 3 transcripts,
 * which is not enough surface to judge a layout. Everything in this block is
 * deterministic (seeded mulberry32, dates anchored to 2026-09-04) and is used
 * only when the real data is too thin; the UI flags it as "sample data".
 * ------------------------------------------------------------------------- */

/** Anchor for every synthetic date so screenshots never drift. */
const DEMO_TODAY = new Date(2026, 8, 4); // 2026-09-04, local midnight.
const DEMO_ACTIVITY_DAYS = 60;
const MIN_REAL_ENTRIES = 5;
const MIN_REAL_ACTIVE_DAYS = 20;
const ACTIVITY_WINDOW_DAYS = 56; // Last 8 weeks.

/** Small deterministic PRNG — no Math.random anywhere in this file. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function isoDay(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function shiftDays(date: Date, days: number): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + days);
}

interface ActivityPoint {
  date: string;
  value: number;
}

/** DEMO ONLY: 60 deterministic words-per-day samples ending 2026-09-04. */
const DEMO_ACTIVITY: ActivityPoint[] = (() => {
  const random = mulberry32(20260904);
  const points: ActivityPoint[] = [];
  for (let offset = DEMO_ACTIVITY_DAYS - 1; offset >= 0; offset -= 1) {
    const date = shiftDays(DEMO_TODAY, -offset);
    const weekend = date.getDay() === 0 || date.getDay() === 6;
    const roll = random();
    const quiet = roll < (weekend ? 0.45 : 0.12);
    const ramp = 0.55 + (DEMO_ACTIVITY_DAYS - offset) / DEMO_ACTIVITY_DAYS * 0.6;
    const base = weekend ? 180 : 620;
    const value = quiet ? 0 : Math.round(base * ramp * (0.45 + random() * 1.05));
    points.push({ date: isoDay(date), value });
  }
  return points;
})();

function demoEntry(
  id: string,
  text: string,
  dayOffset: number,
  hour: number,
  minute: number,
  duration: number,
  source: 'recording' | 'file',
  sourceName?: string,
): HistoryEntry {
  const day = shiftDays(DEMO_TODAY, dayOffset);
  return {
    schemaVersion: 2,
    id,
    text,
    timestamp: new Date(day.getFullYear(), day.getMonth(), day.getDate(), hour, minute).getTime(),
    duration,
    source,
    ...(sourceName ? { sourceName } : {}),
  };
}

/** DEMO ONLY: six transcripts across Today (2026-09-04) and Yesterday. */
const DEMO_ENTRIES: HistoryEntry[] = [
  demoEntry(
    'demo-1',
    'Ship the quiet-focus home first, then fold the same tab row into the meetings workspace so both surfaces read the same way.',
    0, 16, 12, 14, 'recording',
  ),
  demoEntry(
    'demo-2',
    'Reminder for the release notes: transcription stays on the ANE, and nothing in this window ever leaves the Mac.',
    0, 14, 38, 9, 'recording',
  ),
  demoEntry(
    'demo-3',
    'Design review recording — we agreed the right rail should collapse into a single card and the styles tip should retire into the Next line.',
    0, 11, 20, 47, 'file', 'design-review.wav',
  ),
  demoEntry(
    'demo-4',
    'git rebase --interactive origin/main, then squash the two overlay-geometry commits before opening the pull request.',
    0, 9, 5, 8, 'recording',
  ),
  demoEntry(
    'demo-5',
    'Yesterday I dictated the whole onboarding copy in one pass and only had to correct two proper nouns, both of which are now preferred terms.',
    -1, 17, 44, 22, 'recording',
  ),
  demoEntry(
    'demo-6',
    'Standup notes: capture supervisor lands today, benchmark gate reruns tonight, and the notetaker summary prompt still needs a second pass.',
    -1, 9, 32, 31, 'recording',
  ),
];

/* --- Presentation helpers ------------------------------------------------ */

function dayKeyOf(timestamp: number): string {
  return isoDay(new Date(timestamp));
}

function dayLabelOf(timestamp: number, reference: Date): string {
  const key = dayKeyOf(timestamp);
  if (key === isoDay(reference)) return 'Today';
  if (key === isoDay(shiftDays(reference, -1))) return 'Yesterday';
  return new Date(timestamp).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

const MAIN_TABS = [
  { value: 'recent', title: 'Recent' },
  { value: 'meetings', title: 'Meetings' },
  { value: 'queries', title: 'Queries' },
];

const SOURCE_TABS = [
  { value: 'all', title: 'All' },
  { value: 'recording', title: 'Mic' },
  { value: 'file', title: 'File' },
];

/** Murmur chart ramp, so the graph tracks the app tokens instead of the
 *  Sona defaults (which resolve against an undefined `--primary`). */
const ACTIVITY_TOKENS = {
  '--activity-graph-empty': 'var(--ui-chart-0)',
  '--activity-graph-level-1': 'var(--ui-chart-1)',
  '--activity-graph-level-2': 'var(--ui-chart-2)',
  '--activity-graph-level-3': 'var(--ui-chart-3)',
  '--activity-graph-level-4': 'var(--ui-chart-4)',
  '--activity-graph-cell-size': '0.6875rem',
  '--activity-graph-cell-gap': '0.1875rem',
  '--activity-graph-cell-radius': '0.1875rem',
} as CSSProperties;

export function VariantC({
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
  const [tab, setTab] = useState('recent');
  const [sourceFilter, setSourceFilter] = useState<HistoryFilter>('all');
  const [query, setQuery] = useState('');
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // Mock-local only: the shared props contract has no per-entry delete
  // command, so Delete hides the row in this variant rather than pretending.
  const [hiddenIds, setHiddenIds] = useState<readonly string[]>([]);

  const stats = useMemo(() => loadStats(), [statsVersion]);
  const usage = useMemo(() => getUsageOverview(stats), [stats]);
  const personalization = useMemo(
    () => derivePersonalization(settings, stats),
    [settings.vocabularyEntries, settings.appProfiles, stats],
  );

  const usingDemoEntries = historyEntries.length < MIN_REAL_ENTRIES;
  const entries = usingDemoEntries ? DEMO_ENTRIES : historyEntries;
  const referenceDay = usingDemoEntries ? DEMO_TODAY : new Date();

  const realActivity = useMemo<ActivityPoint[]>(() => {
    const points: ActivityPoint[] = [];
    for (let offset = ACTIVITY_WINDOW_DAYS - 1; offset >= 0; offset -= 1) {
      const key = isoDay(shiftDays(new Date(), -offset));
      points.push({ date: key, value: stats.dailyBuckets[key]?.words ?? 0 });
    }
    return points;
  }, [stats]);
  const usingDemoActivity =
    realActivity.filter((point) => point.value > 0).length < MIN_REAL_ACTIVE_DAYS;
  const activityEnd = usingDemoActivity ? DEMO_TODAY : new Date();
  const activityData = usingDemoActivity ? DEMO_ACTIVITY : realActivity;

  const visible = useMemo(
    () => sortForDisplay(filterHistory(entries, { query, filter: sourceFilter }))
      .filter((entry) => !hiddenIds.includes(entry.id)),
    [entries, query, sourceFilter, hiddenIds],
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
  const actionLabel = status === 'starting'
    ? 'Cancel'
    : status === 'recording'
      ? 'Stop recording'
      : status === 'processing'
        ? 'Processing'
        : status === 'recovering'
          ? 'Recovering'
          : 'Start recording';

  const copyEntry = (entry: HistoryEntry) => {
    setCopiedId(entry.id);
    setNotice(null);
    navigator.clipboard?.writeText(entry.text).catch(() => {
      setCopiedId(null);
      setNotice('Could not copy to the clipboard.');
    });
  };

  return (
    <div className="home-dashboard vc-root" data-redesign-variant="C">
      <header className="vc-hero">
        <RippleButton
          className="vc-record"
          data-testid="home-record-button"
          rippleStyle="vc-record-ripple"
          onClick={() => void (isCapturing ? onStop() : onRecord())}
          disabled={!initialized || busy || meetingBusy}
          data-tone={isCapturing ? 'danger' : 'default'}
          aria-label={status === 'recording'
            ? `Stop recording, ${timer(recordingDuration)}`
            : status === 'starting'
              ? 'Cancel recording'
              : busy ? statusTitle : 'Start recording'}
        >
          <span className="vc-record-dot" aria-hidden="true" />
          <RippleButtonText className="vc-record-label" text={actionLabel} />
        </RippleButton>

        <div className="vc-hero-status" aria-live="polite">
          <strong>{statusTitle}</strong>
          <span className="vc-hero-hint">
            {hotkeyVerb(settings.recordingMode)}
            {KEY_CHIPS[settings.doubleTapKey].map((chip) => (
              <kbd key={chip}>{chip}</kbd>
            ))}
            anywhere to begin
          </span>
        </div>

        <button
          type="button"
          className="vc-quiet-link"
          onClick={onTranscribeFile}
          disabled={isCapturing || busy || meetingBusy}
        >
          Transcribe file…
        </button>
      </header>

      <div className="vc-grid">
        <div className="vc-main">
          <div className="vc-tab-row">
            <FluidTabs
              tabs={MAIN_TABS}
              value={tab}
              onValueChange={setTab}
              variant="underline"
              size="md"
              ariaLabel="Transcript collections"
              className="vc-main-tabs"
            />
            <span className="vc-tab-count">
              {tab === 'recent'
                ? `${visible.length} ${visible.length === 1 ? 'transcript' : 'transcripts'}`
                : ''}
              {tab === 'recent' && usingDemoEntries && <em> · sample data</em>}
            </span>
          </div>

          {tab === 'recent' ? (
            <>
              <div className="vc-filter-row">
                <div className="history-search vc-search" data-expanded="true">
                  <span className="vc-search-icon" aria-hidden="true">
                    <svg fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-4.35-4.35M17 11a6 6 0 11-12 0 6 6 0 0112 0z" />
                    </svg>
                  </span>
                  <input
                    type="search"
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder="Search transcripts"
                    aria-label="Search transcripts"
                    className="vc-search-input"
                  />
                </div>
                <FluidTabs
                  tabs={SOURCE_TABS}
                  value={sourceFilter}
                  onValueChange={(value) => setSourceFilter(value as HistoryFilter)}
                  variant="capsule"
                  size="sm"
                  ariaLabel="Filter transcripts by source"
                  className="vc-source-tabs"
                />
              </div>

              {notice && <p role="status" className="vc-notice">{notice}</p>}

              <div className="vc-list">
                {visible.length === 0 ? (
                  <p className="vc-empty">
                    No transcripts match this search. Clear the search box or switch back to All.
                  </p>
                ) : visible.map((entry, index) => {
                  const showDayLabel = index === 0
                    || dayKeyOf(entry.timestamp) !== dayKeyOf(visible[index - 1].timestamp);
                  return (
                    <div key={entry.id} className="vc-row">
                      {showDayLabel && (
                        <p className="history-date-label">{dayLabelOf(entry.timestamp, referenceDay)}</p>
                      )}
                      <article
                        data-testid="transcript-card"
                        data-copied={copiedId === entry.id}
                        className="transcript-card vc-card"
                        role="group"
                        tabIndex={0}
                        aria-label={`Transcription from ${formatTimestamp(entry.timestamp)}. Press Enter or Space to copy.`}
                        onKeyDown={(event) => {
                          if (event.target !== event.currentTarget) return;
                          if (event.key !== 'Enter' && event.key !== ' ') return;
                          event.preventDefault();
                          copyEntry(entry);
                        }}
                      >
                        <div className="transcript-meta">
                          <span className="vc-time">{formatTimestamp(entry.timestamp)}</span>
                          {entrySource(entry) === 'file' ? (
                            <span className="vc-chip" data-source="file" title={entry.sourceName}>
                              {entry.sourceName || 'File'}
                            </span>
                          ) : (
                            <span className="vc-chip" data-source="mic">Mic</span>
                          )}
                          <span className="vc-duration">{timer(entry.duration)}</span>
                        </div>

                        <div className="vc-card-actions">
                          <SmartOverflow
                            ariaLabel={`Actions for the transcript from ${formatTimestamp(entry.timestamp)}`}
                            moreLabel="More transcript actions"
                            className="vc-overflow"
                            actionClassName="vc-overflow-action"
                            moreButtonClassName="vc-overflow-trigger"
                          >
                            <SmartOverflowAction
                              id="copy"
                              priority="primary"
                              icon={<Copy />}
                              onSelect={() => copyEntry(entry)}
                            >
                              {copiedId === entry.id ? 'Copied' : 'Copy'}
                            </SmartOverflowAction>
                            <SmartOverflowAction
                              id="teach"
                              priority="secondary"
                              icon={<GraduationCap />}
                              onSelect={() => setNotice('Correct & Teach opens on the newest real transcript.')}
                            >
                              Correct &amp; Teach
                            </SmartOverflowAction>
                            <SmartOverflowAction
                              id="delete"
                              priority="overflow"
                              destructive
                              icon={<Trash2 />}
                              onSelect={() => setHiddenIds((ids) => [...ids, entry.id])}
                            >
                              Delete
                            </SmartOverflowAction>
                          </SmartOverflow>
                        </div>

                        <p className="transcript-text vc-text">{entry.text}</p>
                      </article>
                    </div>
                  );
                })}
              </div>
            </>
          ) : (
            <p className="vc-empty vc-empty-tab">
              {tab === 'meetings'
                ? 'No meetings yet. Start the notetaker and finished sessions land here.'
                : 'No queries yet. Ask a question with the query hotkey and the answers stay here.'}
            </p>
          )}
        </div>

        <aside className="vc-rail" aria-label="Usage summary">
          <DashboardSurface as="section" variant="outlined" padding="standard" ariaLabel="Usage and personalization">
            <p className="ui-dashboard-eyebrow vc-rail-eyebrow">
              Last 8 weeks{usingDemoActivity && <em> · sample</em>}
            </p>
            <ActivityGraph
              className="vc-activity"
              data={activityData}
              startDate={isoDay(shiftDays(activityEnd, -(ACTIVITY_WINDOW_DAYS - 1)))}
              endDate={isoDay(activityEnd)}
              maxDays={ACTIVITY_WINDOW_DAYS}
              levels={4}
              weekStartsOn={1}
              showValue={false}
              showLegend={false}
              showWeekdayLabels={false}
              showMonthLabels
              ariaLabel="Words dictated per day over the last eight weeks"
              style={ACTIVITY_TOKENS}
            />

            <dl className="vc-stats" aria-label="Usage this month">
              <div>
                <dt>Words this month</dt>
                <dd>{usage.wordsThisMonth.toLocaleString()}</dd>
              </div>
              <div>
                <dt>Day streak</dt>
                <dd>{usage.currentStreak}</dd>
              </div>
            </dl>

            <div className="vc-next">
              <p><strong>Next</strong> · {personalization.nextAction}</p>
              <DashboardAction
                variant="quiet"
                onActivate={() => onOpenSettings({ page: 'delivery', target: 'app-overrides' })}
              >
                Set up styles
              </DashboardAction>
            </div>

            <div className="vc-rail-foot">
              <DashboardAction variant="quiet" icon="forward" onActivate={onOpenInsights}>
                View insights
              </DashboardAction>
            </div>
          </DashboardSurface>
        </aside>
      </div>
    </div>
  );
}
