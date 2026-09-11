import type { KnowledgeScope } from './knowledge';

export interface TransformCounts {
  runs: number;
  approved: number;
  undone: number;
}

export interface ActivityMonth extends TransformCounts {
  meetings: number;
  meetingMinutes: number;
}

export interface ActivityStats {
  meetings: { count: number; totalMinutes: number; summariesGenerated: number };
  transforms: TransformCounts & { byPresetName: Record<string, TransformCounts> };
  corrections: {
    proposed: number;
    taught: number;
    byScope: Record<KnowledgeScope['kind'], number>;
  };
  recordingsByMode: Record<string, number>;
  pasteLastUses: number;
  monthlyBuckets: Record<string, ActivityMonth>;
}

export type ActivityCompletion =
  | { kind: 'meeting'; durationMs: number }
  | { kind: 'meeting_summary' }
  | { kind: 'transform'; outcome: keyof TransformCounts; presetName: string | null }
  | { kind: 'correction_proposed' }
  | { kind: 'correction_taught'; scope: KnowledgeScope['kind'] }
  | { kind: 'paste_last' };

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

function count(value: unknown): number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : 0;
}

function duration(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : 0;
}

function transformCounts(value: unknown): TransformCounts {
  const raw = record(value);
  return { runs: count(raw.runs), approved: count(raw.approved), undone: count(raw.undone) };
}

export function activityMonth(value?: unknown): ActivityMonth {
  const raw = record(value);
  return { ...transformCounts(raw), meetings: count(raw.meetings), meetingMinutes: duration(raw.meetingMinutes) };
}

// Mode IDs and preset names are the only private labels accepted here. Never
// pass the stats blob or these keys to diagnostics, logs, or telemetry.
export function sanitizeActivityStats(value: unknown): ActivityStats {
  const raw = record(value);
  const meetings = record(raw.meetings);
  const transforms = record(raw.transforms);
  const corrections = record(raw.corrections);
  const scopes = record(corrections.byScope);
  return {
    meetings: {
      count: count(meetings.count),
      totalMinutes: duration(meetings.totalMinutes),
      summariesGenerated: count(meetings.summariesGenerated),
    },
    transforms: {
      ...transformCounts(transforms),
      byPresetName: Object.fromEntries(Object.entries(record(transforms.byPresetName))
        .filter(([name]) => name.length > 0)
        .map(([name, totals]) => [name, transformCounts(totals)])),
    },
    corrections: {
      proposed: count(corrections.proposed),
      taught: count(corrections.taught),
      byScope: { global: count(scopes.global), app: count(scopes.app), project: count(scopes.project) },
    },
    recordingsByMode: Object.fromEntries(Object.entries(record(raw.recordingsByMode))
      .filter(([id]) => id.length > 0)
      .map(([id, total]) => [id, count(total)])),
    pasteLastUses: count(raw.pasteLastUses),
    monthlyBuckets: Object.fromEntries(Object.entries(record(raw.monthlyBuckets))
      .filter(([month]) => /^\d{4}-(0[1-9]|1[0-2])$/.test(month))
      .map(([month, totals]) => [month, activityMonth(totals)])),
  };
}

export function monthKey(date: Date = new Date()): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}`;
}

export function addActivityCompletion(
  current: ActivityStats,
  completion: ActivityCompletion,
  month = monthKey(),
): ActivityStats {
  const next = sanitizeActivityStats(current);
  switch (completion.kind) {
    case 'meeting': {
      const minutes = duration(completion.durationMs) / 60_000;
      next.meetings.count++;
      next.meetings.totalMinutes += minutes;
      const bucket = activityMonth(next.monthlyBuckets[month]);
      bucket.meetings++;
      bucket.meetingMinutes += minutes;
      next.monthlyBuckets[month] = bucket;
      break;
    }
    case 'meeting_summary': next.meetings.summariesGenerated++; break;
    case 'transform': {
      next.transforms[completion.outcome]++;
      const bucket = activityMonth(next.monthlyBuckets[month]);
      bucket[completion.outcome]++;
      next.monthlyBuckets[month] = bucket;
      if (completion.presetName !== null) {
        const byPresetName = next.transforms.byPresetName;
        const previous = Object.prototype.hasOwnProperty.call(byPresetName, completion.presetName)
          ? byPresetName[completion.presetName] : undefined;
        const totals = transformCounts(previous);
        totals[completion.outcome]++;
        next.transforms.byPresetName = { ...byPresetName, [completion.presetName]: totals };
      }
      break;
    }
    case 'correction_proposed': next.corrections.proposed++; break;
    case 'correction_taught':
      next.corrections.taught++;
      next.corrections.byScope[completion.scope]++;
      break;
    case 'paste_last': next.pasteLastUses++; break;
    default: {
      const exhaustive: never = completion;
      return exhaustive;
    }
  }
  return next;
}

export const ASSUMED_TYPING_WPM = 40;

export function getActivityInsights(stats: {
  totalWords: number;
  totalDurationSeconds: number;
  activity: ActivityStats;
}, now = new Date()) {
  const month = activityMonth(stats.activity.monthlyBuckets[monthKey(now)]);
  const mostUsedMode = Object.entries(stats.activity.recordingsByMode)
    .filter(([, count]) => count > 0)
    .sort(([idA, countA], [idB, countB]) => countB - countA || idA.localeCompare(idB))[0];
  const mostUsedTransform = Object.entries(stats.activity.transforms.byPresetName)
    .filter(([, counts]) => counts.runs > 0)
    .sort(([nameA, countsA], [nameB, countsB]) => countsB.runs - countsA.runs || nameA.localeCompare(nameB))[0];
  return {
    dictationMinutes: stats.totalDurationSeconds / 60,
    typingMinutesSaved: Math.max(0, stats.totalWords / ASSUMED_TYPING_WPM - stats.totalDurationSeconds / 60),
    mostUsedModeId: mostUsedMode?.[0] ?? null,
    mostUsedModeRecordings: mostUsedMode?.[1] ?? 0,
    mostUsedTransformName: mostUsedTransform?.[0] ?? null,
    mostUsedTransformRuns: mostUsedTransform?.[1].runs ?? 0,
    month,
    approvalRate: month.runs > 0 ? Math.round(month.approved / month.runs * 100) : 0,
  };
}
