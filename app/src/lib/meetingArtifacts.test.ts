import { describe, expect, it } from 'vitest';
import { chunkMeetingSegments, formatMeetingArtifactExport, MEETING_ARTIFACT_SCHEMA, mergeMeetingArtifacts, parseMeetingArtifact, type MeetingArtifactV1 } from './meetingArtifacts';
import type { MeetingSegment } from './meetings';

const segment = (id: number, text = `segment ${id}`): MeetingSegment => ({ id, sessionId: 's', speaker: 'me', sequence: id, startMs: id * 1000, endMs: id * 1000 + 500, status: 'final', text, audioAvailable: false, errorCode: null });
const artifact = (id: number): MeetingArtifactV1 => ({ schema: MEETING_ARTIFACT_SCHEMA, summary: { text: `summary ${id}`, sourceSegmentIds: [id] }, decisions: [{ text: `decision ${id}`, sourceSegmentIds: [id] }], actionItems: [{ text: `action ${id}`, owner: null, dueDate: null, sourceSegmentIds: [id] }], openQuestions: [] });

describe('meeting artifact foundation', () => {
  it('chunks long meetings with bounded segment and character limits', () => {
    const chunks = chunkMeetingSegments(Array.from({ length: 120 }, (_, i) => segment(i + 1, 'x'.repeat(300))), 1000);
    expect(chunks.length).toBeGreaterThan(3);
    expect(chunks.every((chunk) => chunk.length <= 50)).toBe(true);
  });
  it('ignores pending, failed, and empty transcript segments', () => {
    expect(chunkMeetingSegments([{ ...segment(1), status: 'pending' }, segment(2, ' ')])).toEqual([]);
  });
  it('accepts strict sourced artifacts and preserves unknown owner/date', () => {
    const parsed = parseMeetingArtifact(artifact(1), [1]);
    expect(parsed?.actionItems[0]).toMatchObject({ owner: null, dueDate: null });
  });
  it('normalizes unsupported owner and date claims back to unknown', () => {
    const value = artifact(1) as unknown as { actionItems: Array<Record<string, unknown>> };
    value.actionItems[0].owner = '';
    value.actionItems[0].dueDate = 'next Friday';
    expect(parseMeetingArtifact(value, [1])?.actionItems[0]).toMatchObject({ owner: null, dueDate: null });
  });
  it('rejects hallucinated source segment IDs', () => {
    expect(parseMeetingArtifact(artifact(99), [1, 2])).toBeNull();
  });
  it('rejects malformed and oversized output deterministically', () => {
    expect(parseMeetingArtifact({ schema: MEETING_ARTIFACT_SCHEMA }, [1])).toBeNull();
    expect(parseMeetingArtifact({ ...artifact(1), decisions: Array(201).fill(artifact(1).decisions[0]) }, [1])).toBeNull();
    expect(parseMeetingArtifact({ ...artifact(1), inventedTopic: 'not in schema' }, [1])).toBeNull();
  });
  it('hierarchically merges beyond one fan-in level and deduplicates claims', () => {
    const merged = mergeMeetingArtifacts(Array.from({ length: 70 }, (_, i) => artifact((i % 10) + 1)));
    expect(merged?.summary.sourceSegmentIds).toHaveLength(10);
    expect(merged?.decisions).toHaveLength(10);
    expect(merged?.actionItems.every((item) => item.owner === null)).toBe(true);
  });
  it('exports markdown, plain text, and self-identifying JSON', () => {
    const value = artifact(1);
    expect(formatMeetingArtifactExport(value, 'markdown')).toContain('## Action items');
    expect(formatMeetingArtifactExport(value, 'text')).toContain('Owner: Unknown; Due: Unknown');
    expect(JSON.parse(formatMeetingArtifactExport(value, 'json')).schema).toBe(MEETING_ARTIFACT_SCHEMA);
  });
});
