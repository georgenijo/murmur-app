import type { MeetingSegment } from './meetings';

export const MEETING_ARTIFACT_SCHEMA = 'murmur.meeting-artifact.v1' as const;
export const MAX_ARTIFACT_ITEMS = 200;
export const MAX_ARTIFACT_TEXT = 16_384;
export const DEFAULT_CHUNK_CHARS = 12_000;
export const MAX_CHUNK_SEGMENTS = 50;
export const MERGE_FAN_IN = 8;

export interface SourcedMeetingText { text: string; sourceSegmentIds: number[] }
export interface MeetingActionItem extends SourcedMeetingText { owner: string | null; dueDate: string | null }
export interface MeetingArtifactV1 {
  schema: typeof MEETING_ARTIFACT_SCHEMA;
  summary: SourcedMeetingText;
  decisions: SourcedMeetingText[];
  actionItems: MeetingActionItem[];
  openQuestions: SourcedMeetingText[];
}
export type MeetingArtifactExportFormat = 'markdown' | 'text' | 'json';

function exactKeys(value: Record<string, unknown>, expected: string[]): boolean {
  const keys = Object.keys(value).sort();
  return keys.length === expected.length && keys.every((key, index) => key === [...expected].sort()[index]);
}

export function chunkMeetingSegments(
  segments: MeetingSegment[], maxChars = DEFAULT_CHUNK_CHARS,
): MeetingSegment[][] {
  const limit = Math.max(256, Math.min(DEFAULT_CHUNK_CHARS, Math.trunc(maxChars)));
  const chunks: MeetingSegment[][] = [];
  let current: MeetingSegment[] = [];
  let chars = 0;
  for (const segment of [...segments].sort((a, b) => a.sequence - b.sequence)) {
    if (segment.status !== 'final' || !segment.text.trim()) continue;
    const size = Math.min(segment.text.length, MAX_ARTIFACT_TEXT);
    if (current.length && (current.length >= MAX_CHUNK_SEGMENTS || chars + size > limit)) {
      chunks.push(current); current = []; chars = 0;
    }
    current.push(segment); chars += size;
  }
  if (current.length) chunks.push(current);
  return chunks;
}

function sourced(value: unknown, allowed: Set<number>): SourcedMeetingText | null {
  if (!value || typeof value !== 'object') return null;
  const row = value as Record<string, unknown>;
  if (!exactKeys(row, ['text', 'sourceSegmentIds']) && !exactKeys(row, ['text', 'sourceSegmentIds', 'owner', 'dueDate'])) return null;
  if (typeof row.text !== 'string' || !row.text.trim() || row.text.length > MAX_ARTIFACT_TEXT) return null;
  if (!Array.isArray(row.sourceSegmentIds) || row.sourceSegmentIds.length === 0) return null;
  const ids = row.sourceSegmentIds;
  if (!ids.every((id) => Number.isSafeInteger(id) && (id as number) > 0 && allowed.has(id as number))) return null;
  return { text: row.text.trim(), sourceSegmentIds: [...new Set(ids as number[])] };
}

export function parseMeetingArtifact(value: unknown, allowedSegmentIds: Iterable<number>): MeetingArtifactV1 | null {
  if (!value || typeof value !== 'object') return null;
  const row = value as Record<string, unknown>;
  if (!exactKeys(row, ['schema', 'summary', 'decisions', 'actionItems', 'openQuestions'])) return null;
  if (row.schema !== MEETING_ARTIFACT_SCHEMA) return null;
  const allowed = new Set(allowedSegmentIds);
  const summary = sourced(row.summary, allowed);
  if (!summary || !Array.isArray(row.decisions) || !Array.isArray(row.actionItems) || !Array.isArray(row.openQuestions)) return null;
  if ([row.decisions, row.actionItems, row.openQuestions].some((items) => (items as unknown[]).length > MAX_ARTIFACT_ITEMS)) return null;
  const decisions = row.decisions.map((item) => sourced(item, allowed));
  const openQuestions = row.openQuestions.map((item) => sourced(item, allowed));
  const actionItems = row.actionItems.map((item) => {
    const base = sourced(item, allowed);
    if (!base || !item || typeof item !== 'object') return null;
    const action = item as Record<string, unknown>;
    if (action.owner !== null && typeof action.owner !== 'string') return null;
    if (action.dueDate !== null && typeof action.dueDate !== 'string') return null;
    const owner = action.owner === null ? null : typeof action.owner === 'string' && action.owner.trim() ? action.owner.trim() : null;
    const dueDate = action.dueDate === null ? null : typeof action.dueDate === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(action.dueDate) ? action.dueDate : null;
    return { ...base, owner, dueDate };
  });
  if ([...decisions, ...openQuestions, ...actionItems].some((item) => item === null)) return null;
  return {
    schema: MEETING_ARTIFACT_SCHEMA, summary,
    decisions: decisions as SourcedMeetingText[],
    actionItems: actionItems as MeetingActionItem[],
    openQuestions: openQuestions as SourcedMeetingText[],
  };
}

function dedupe<T extends SourcedMeetingText>(items: T[]): T[] {
  const seen = new Set<string>();
  return items.filter((item) => {
    const key = item.text.toLocaleLowerCase();
    if (seen.has(key)) return false;
    seen.add(key); return true;
  }).slice(0, MAX_ARTIFACT_ITEMS);
}

export function mergeMeetingArtifacts(artifacts: MeetingArtifactV1[]): MeetingArtifactV1 | null {
  if (!artifacts.length) return null;
  let level = artifacts;
  while (level.length > 1) {
    const next: MeetingArtifactV1[] = [];
    for (let index = 0; index < level.length; index += MERGE_FAN_IN) {
      const group = level.slice(index, index + MERGE_FAN_IN);
      const summaryIds = [...new Set(group.flatMap((item) => item.summary.sourceSegmentIds))];
      next.push({
        schema: MEETING_ARTIFACT_SCHEMA,
        summary: { text: group.map((item) => item.summary.text).join(' ').slice(0, MAX_ARTIFACT_TEXT), sourceSegmentIds: summaryIds },
        decisions: dedupe(group.flatMap((item) => item.decisions)),
        actionItems: dedupe(group.flatMap((item) => item.actionItems)),
        openQuestions: dedupe(group.flatMap((item) => item.openQuestions)),
      });
    }
    level = next;
  }
  return level[0];
}

export function formatMeetingArtifactExport(artifact: MeetingArtifactV1, format: MeetingArtifactExportFormat): string {
  if (format === 'json') return `${JSON.stringify(artifact, null, 2)}\n`;
  const sources = (item: SourcedMeetingText) => ` [segments: ${item.sourceSegmentIds.join(', ')}]`;
  const action = (item: MeetingActionItem) => `${item.text} — Owner: ${item.owner ?? 'Unknown'}; Due: ${item.dueDate ?? 'Unknown'}${sources(item)}`;
  if (format === 'markdown') {
    return `# Meeting summary\n\n${artifact.summary.text}${sources(artifact.summary)}\n\n## Decisions\n${artifact.decisions.map((x) => `- ${x.text}${sources(x)}`).join('\n') || '- None'}\n\n## Action items\n${artifact.actionItems.map((x) => `- ${action(x)}`).join('\n') || '- None'}\n\n## Open questions\n${artifact.openQuestions.map((x) => `- ${x.text}${sources(x)}`).join('\n') || '- None'}\n`;
  }
  return `MEETING SUMMARY\n${artifact.summary.text}${sources(artifact.summary)}\n\nDECISIONS\n${artifact.decisions.map((x) => `- ${x.text}${sources(x)}`).join('\n') || '- None'}\n\nACTION ITEMS\n${artifact.actionItems.map((x) => `- ${action(x)}`).join('\n') || '- None'}\n\nOPEN QUESTIONS\n${artifact.openQuestions.map((x) => `- ${x.text}${sources(x)}`).join('\n') || '- None'}\n`;
}
