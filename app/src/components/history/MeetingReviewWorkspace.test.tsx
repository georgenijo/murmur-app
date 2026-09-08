import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import type { MeetingDetail, MeetingSegment } from '../../lib/meetings';
import { MeetingReviewWorkspace } from './MeetingReviewWorkspace';

const segments: MeetingSegment[] = [
  { id: 11, sessionId: 'meeting', speaker: 'me', remoteSpeakerId: null, sequence: 0, startMs: 1_000, endMs: 2_000, status: 'final', text: 'Raw evidence', audioAvailable: false, errorCode: null },
  { id: 12, sessionId: 'meeting', speaker: 'them', remoteSpeakerId: 1, sequence: 0, startMs: 2_000, endMs: 3_000, status: 'final', text: 'Remote evidence', audioAvailable: false, errorCode: null },
  { id: 13, sessionId: 'meeting', speaker: 'them', remoteSpeakerId: null, sequence: 1, startMs: 3_000, endMs: 4_000, status: 'final', text: 'Uncertain evidence', audioAvailable: false, errorCode: null },
];

const detail: MeetingDetail = {
  session: { id: 'meeting', startedAtMs: 1, endedAtMs: 2, status: 'complete', modelName: 'base.en', language: 'en', smartPunctuation: true, retainAudio: false, durationMs: 1_000, segmentCount: 1, preview: 'Raw evidence', errorCode: null },
  segments,
  labels: { me: 'George', them: 'Team' },
  remoteSpeakers: [{ speakerId: 1, label: 'Casey' }],
  generated: { revision: 2, document: { schema: 'murmur.meeting-review.v1', summary: { key: 'summary', text: 'Generated', sourceSegmentIds: [11] }, decisions: [], actionItems: [], openQuestions: [] } },
  review: { revision: 1, basedOnGeneratedRevision: 1, document: { schema: 'murmur.meeting-review.v1', summary: { key: 'summary', text: 'Reviewed', sourceSegmentIds: [11] }, decisions: [], actionItems: [], openQuestions: [] } },
  activeDocument: { schema: 'murmur.meeting-review.v1', summary: { key: 'summary', text: 'Reviewed', sourceSegmentIds: [11] }, decisions: [], actionItems: [], openQuestions: [] },
  activeOrigin: 'reviewed',
};

function controller(overrides: Partial<ReturnType<typeof useMeetings>> = {}): ReturnType<typeof useMeetings> {
  return {
    detail,
    summaryStatus: { generation: 0, sessionId: null, phase: 'idle', completedChunks: 0, totalChunks: 0, elapsedMs: 0, peakRssMb: 0, errorCode: null },
    saveReview: vi.fn().mockResolvedValue(true),
    restoreReview: vi.fn().mockResolvedValue(true),
    copy: vi.fn().mockResolvedValue(true),
    exportReview: vi.fn().mockResolvedValue('/tmp/review.md'),
    summarize: vi.fn().mockResolvedValue(undefined),
    cancelSummary: vi.fn().mockResolvedValue(undefined),
    renameRemoteSpeaker: vi.fn().mockResolvedValue(true),
    ...overrides,
  } as unknown as ReturnType<typeof useMeetings>;
}

describe('MeetingReviewWorkspace', () => {
  let root: Root;
  let container: HTMLDivElement;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    Element.prototype.scrollIntoView = vi.fn();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('moves focus from a sourced claim to immutable transcript evidence', async () => {
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller()} segments={segments} captureBusy={false} onNotice={() => {}} />));

    await act(async () => (container.querySelector('[aria-label^="Summary source"]') as HTMLButtonElement).click());

    expect(document.activeElement).toBe(container.querySelector('#meeting-segment-11'));
    expect(Element.prototype.scrollIntoView).toHaveBeenCalled();
  });

  it('submits editable values without exposing source IDs to the client request', async () => {
    const saveReview = vi.fn().mockResolvedValue(true);
    const meetings = controller({ saveReview });
    await act(async () => root.render(<MeetingReviewWorkspace meetings={meetings} segments={segments} captureBusy={false} onNotice={() => {}} />));
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Edit review')!.click());
    const summary = container.querySelector('[aria-label="Review summary"]') as HTMLTextAreaElement;
    const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
    await act(async () => {
      setValue.call(summary, 'Edited by the reviewer');
      summary.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Save review')!.click());

    expect(saveReview).toHaveBeenCalledWith(expect.objectContaining({
      base: { kind: 'review', reviewRevision: 1 },
      document: expect.objectContaining({ summary: { key: 'summary', text: 'Edited by the reviewer' } }),
    }));
    expect(JSON.stringify(saveReview.mock.calls[0][0])).not.toContain('sourceSegmentIds');
  });

  it('saves labels without turning the generated draft into a reviewed snapshot', async () => {
    const saveReview = vi.fn().mockResolvedValue(true);
    const generatedOnly = { ...detail, review: null, activeDocument: detail.generated!.document, activeOrigin: 'generated' as const };
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ detail: generatedOnly, saveReview })} segments={segments} captureBusy={false} onNotice={() => {}} />));

    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Save labels')!.click());

    expect(saveReview).toHaveBeenCalledWith({
      sessionId: 'meeting',
      expectedReviewRevision: null,
      base: { kind: 'labels_only' },
      labels: { me: 'George', them: 'Team' },
      document: null,
    });
  });

  it('requires a second explicit action before replacing a review from a generated draft', async () => {
    const restoreReview = vi.fn().mockResolvedValue(true);
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ restoreReview })} segments={segments} captureBusy={false} onNotice={() => {}} />));
    const restore = () => container.querySelector('[aria-label="Replace review with generated draft"]') as HTMLButtonElement;

    await act(async () => restore().click());
    expect(restoreReview).not.toHaveBeenCalled();
    await act(async () => restore().click());
    expect(restoreReview).toHaveBeenCalledWith('meeting', 2, 1);
  });

  it('shows resolved remote names, keeps uncertain Them fallback, and renames one session speaker', async () => {
    const renameRemoteSpeaker = vi.fn().mockResolvedValue(true);
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ renameRemoteSpeaker })} segments={segments} captureBusy={false} onNotice={() => {}} />));

    expect(container.querySelector('#meeting-segment-12')?.textContent).toContain('Casey');
    expect(container.querySelector('#meeting-segment-13')?.textContent).toContain('Team');
    const input = container.querySelector('[aria-label="Remote speaker 1 label"]') as HTMLInputElement;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setValue.call(input, 'Alex');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const save = input.parentElement?.querySelector('button') as HTMLButtonElement;
    await act(async () => save.click());

    expect(renameRemoteSpeaker).toHaveBeenCalledWith('meeting', 1, 'Alex');
  });
});
