import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import type { MeetingAudioController } from '../../lib/hooks/useMeetingAudio';
import type { MeetingDetail } from '../../lib/meetings';

const audioMocks = vi.hoisted(() => ({
  controller: null as MeetingAudioController | null,
}));

vi.mock('../../lib/hooks/useMeetingAudio', () => ({
  useMeetingAudio: () => audioMocks.controller,
}));

import { MeetingsPanel } from './MeetingsPanel';

const detail: MeetingDetail = {
  session: {
    id: 'retained-meeting', title: 'Synthetic retained meeting', titleSource: 'manual', attendees: [],
    startedAtMs: 1, endedAtMs: 4_001, status: 'complete', modelName: 'base.en', language: 'en',
    smartPunctuation: true, retainAudio: true, durationMs: 4_000, segmentCount: 1,
    preview: 'Retained evidence', errorCode: null,
  },
  segments: [{
    id: 11, sessionId: 'retained-meeting', speaker: 'me', remoteSpeakerId: null, sequence: 0,
    startMs: 1_000, endMs: 2_000, status: 'final', text: 'Retained evidence',
    audioAvailable: true, errorCode: null,
  }],
  labels: { me: 'Me', them: 'Them' },
  remoteSpeakers: [], generated: null, review: null, activeDocument: null, activeOrigin: null,
};

function audioController(invalidate: () => void): MeetingAudioController {
  return {
    status: 'ready', unavailableReason: null, error: null, durationMs: 4_000,
    positionMs: 0, channel: 'all', playAll: vi.fn(), playSegment: vi.fn(), play: vi.fn(),
    pause: vi.fn(), seek: vi.fn(), setChannel: vi.fn(), invalidate, retry: vi.fn(),
  };
}

function meetingsController(remove: (id: string) => Promise<boolean>): ReturnType<typeof useMeetings> {
  return {
    status: {
      generation: 0, sessionId: null, phase: 'idle', elapsedMs: 0, microphoneActive: false,
      systemAudioActive: false, echoCancellation: { state: 'off' }, errorCode: null,
    },
    permission: 'granted', access: null,
    page: { sessions: [detail.session], searchMatches: {}, total: 1, offset: 0, limit: 50 },
    detail, appliedQuery: '', liveSegments: [], loading: false, error: null,
    summaryStatus: {
      generation: 0, sessionId: null, phase: 'idle', completedChunks: 0, totalChunks: 0,
      elapsedMs: 0, peakRssMb: 0, errorCode: null,
    },
    refresh: vi.fn(), select: vi.fn(), start: vi.fn(), stop: vi.fn(), requestPermission: vi.fn(),
    openSystemAudioPreferences: vi.fn(), copy: vi.fn(), exportReview: vi.fn(), saveReview: vi.fn(),
    saveMetadata: vi.fn(), applyCalendarEvent: vi.fn(), restoreReview: vi.fn(),
    renameRemoteSpeaker: vi.fn(), remove, clear: vi.fn(), summarize: vi.fn(), cancelSummary: vi.fn(),
  };
}

describe('MeetingsPanel retained audio deletion', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('shows content-free matching field labels and keeps the result selectable', async () => {
    const meetings = meetingsController(vi.fn());
    meetings.appliedQuery = 'reviewonly';
    meetings.page.searchMatches = { [detail.session.id]: ['decision', 'action_item'] };
    await act(async () => root.render(<MeetingsPanel meetings={meetings} playbackBusy={false} />));
    expect(container.textContent).toContain('Matches: Decision, Action item');
    const result = container.querySelector<HTMLButtonElement>('button[aria-pressed]')!;
    await act(async () => result.click());
    expect(meetings.select).toHaveBeenCalledWith(detail.session.id);
  });

  it('distinguishes an empty search from an empty meeting history', async () => {
    const meetings = meetingsController(vi.fn());
    meetings.appliedQuery = 'missing';
    meetings.page = { sessions: [], searchMatches: {}, total: 0, offset: 0, limit: 50 };
    await act(async () => root.render(<MeetingsPanel meetings={meetings} playbackBusy={false} />));
    expect(container.textContent).toContain('No meetings match this search.');
    expect(container.textContent).not.toContain('No meeting transcripts yet.');
    meetings.appliedQuery = '';
    await act(async () => root.render(<MeetingsPanel meetings={meetings} playbackBusy={false} />));
    expect(container.textContent).toContain('No meeting transcripts yet.');
  });

  it('invalidates local playback before deleting the selected meeting', async () => {
    const order: string[] = [];
    const remove = vi.fn(async () => {
      order.push('remove');
      return true;
    });
    audioMocks.controller = audioController(() => { order.push('invalidate'); });
    await act(async () => root.render(
      <MeetingsPanel meetings={meetingsController(remove)} playbackBusy={false} />,
    ));

    const click = async (label: string) => {
      const button = [...container.querySelectorAll('button')].find(
        (candidate) => candidate.textContent?.trim() === label,
      );
      if (!button) throw new Error(`Missing ${label} button`);
      await act(async () => button.click());
    };
    await click('Delete');
    await click('Confirm Delete');

    expect(order).toEqual(['invalidate', 'remove']);
    expect(remove).toHaveBeenCalledWith('retained-meeting');
  });
});
