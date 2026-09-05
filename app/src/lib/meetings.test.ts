import { describe, expect, it } from 'vitest';
import {
  echoCancellationNotice,
  formatMeetingTimestamp,
  orderedMeetingSegments,
  type MeetingSegment,
} from './meetings';

function segment(id: number, startMs: number): MeetingSegment {
  return {
    id,
    sessionId: 'session',
    speaker: id % 2 === 0 ? 'me' : 'them',
    sequence: id,
    startMs,
    endMs: startMs + 500,
    status: 'final',
    text: `segment ${id}`,
    audioAvailable: false,
    errorCode: null,
  };
}

describe('meeting presentation', () => {
  it('orders cross-channel completions by capture time and bounds the live window', () => {
    const ordered = orderedMeetingSegments([
      segment(4, 4_000),
      segment(1, 1_000),
      segment(3, 3_000),
      segment(2, 2_000),
    ], 3);
    expect(ordered.map((item) => item.id)).toEqual([2, 3, 4]);
  });

  it('deduplicates an event that races a detail refresh', () => {
    const newer = { ...segment(2, 2_000), text: 'committed text' };
    const ordered = orderedMeetingSegments([segment(1, 1_000), segment(2, 2_000), newer], 10);
    expect(ordered.map((item) => item.id)).toEqual([1, 2]);
    expect(ordered[1].text).toBe('committed text');
  });

  it('formats long meeting-relative timestamps', () => {
    expect(formatMeetingTimestamp(62_000)).toBe('1:02');
    expect(formatMeetingTimestamp(3_662_000)).toBe('1:01:02');
  });

  it('distinguishes temporary echo recovery from terminal raw-audio fallback', () => {
    expect(echoCancellationNotice({
      state: 'recovering',
      reason: 'renderDiscontinuity',
      attempt: 2,
      maxAttempts: 3,
    })).toContain('attempt 2 of 3');
    expect(echoCancellationNotice({
      state: 'bypassed',
      reason: 'processingBacklog',
    })).toContain('rest of this meeting');
    expect(echoCancellationNotice({ state: 'active' })).toBeNull();
  });
});
