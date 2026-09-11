import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import type { MeetingDetail, MeetingSegment } from '../../lib/meetings';
import { MeetingReviewWorkspace } from './MeetingReviewWorkspace';
import type { MeetingAudioController } from '../../lib/hooks/useMeetingAudio';

const segments: MeetingSegment[] = [
  { id: 11, sessionId: 'meeting', speaker: 'me', remoteSpeakerId: null, sequence: 0, startMs: 1_000, endMs: 2_000, status: 'final', text: 'Raw evidence', audioAvailable: false, errorCode: null },
  { id: 12, sessionId: 'meeting', speaker: 'them', remoteSpeakerId: 1, sequence: 0, startMs: 2_000, endMs: 3_000, status: 'final', text: 'Remote evidence', audioAvailable: false, errorCode: null },
  { id: 13, sessionId: 'meeting', speaker: 'them', remoteSpeakerId: null, sequence: 1, startMs: 3_000, endMs: 4_000, status: 'final', text: 'Uncertain evidence', audioAvailable: false, errorCode: null },
];

const detail: MeetingDetail = {
  session: { id: 'meeting', title: null, titleSource: null, attendees: [], startedAtMs: 1, endedAtMs: 2, status: 'complete', modelName: 'base.en', language: 'en', smartPunctuation: true, retainAudio: false, durationMs: 1_000, segmentCount: 1, preview: 'Raw evidence', errorCode: null },
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
    saveMetadata: vi.fn().mockResolvedValue(true),
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

function audioController(overrides: Partial<MeetingAudioController> = {}): MeetingAudioController {
  return {
    status: 'ready',
    unavailableReason: null,
    error: null,
    durationMs: 4_000,
    positionMs: 0,
    channel: 'all',
    playAll: vi.fn(),
    playSegment: vi.fn(),
    play: vi.fn(),
    pause: vi.fn(),
    seek: vi.fn(),
    setChannel: vi.fn(),
    invalidate: vi.fn(),
    retry: vi.fn(),
    ...overrides,
  };
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

  it('saves a manual title and attendees without changing the review', async () => {
    const saveMetadata = vi.fn().mockResolvedValue(true);
    const saveReview = vi.fn();
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ saveMetadata, saveReview })} segments={segments} captureBusy={false} onNotice={() => {}} />));
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Name meeting')?.click());
    const title = container.querySelector<HTMLInputElement>('form[aria-label="Name meeting"] input');
    const attendees = container.querySelector<HTMLTextAreaElement>('form[aria-label="Name meeting"] textarea');
    if (!title || !attendees) throw new Error('Missing meeting metadata fields');
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(title, '  Planning session  ');
      title.dispatchEvent(new Event('input', { bubbles: true }));
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(attendees, 'Alex\n\n Casey ');
      attendees.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Save details')?.click());
    expect(saveMetadata).toHaveBeenCalledWith({ sessionId: 'meeting', title: 'Planning session', attendees: ['Alex', 'Casey'] });
    expect(saveReview).not.toHaveBeenCalled();
  });

  it('keeps naming input after a failed save and clears it when selecting another meeting', async () => {
    const saveMetadata = vi.fn().mockResolvedValue(false);
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ saveMetadata })} segments={segments} captureBusy={false} onNotice={() => {}} />));
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Name meeting')?.click());
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Save details')?.click());
    expect(container.querySelector('form[aria-label="Name meeting"]')).not.toBeNull();
    const other: MeetingDetail = { ...detail, session: { ...detail.session, id: 'other', title: 'Other meeting', titleSource: 'manual' } };
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ detail: other, saveMetadata })} segments={segments} captureBusy={false} onNotice={() => {}} />));
    expect(container.querySelector('form[aria-label="Name meeting"]')).toBeNull();
    expect(container.textContent).toContain('Other meeting');
  });

  it('copies and exports captions with the selected format and explains their scope', async () => {
    const meetings = controller();
    const onNotice = vi.fn();
    await act(async () => root.render(<MeetingReviewWorkspace meetings={meetings} segments={segments} captureBusy={false} onNotice={onNotice} />));
    const select = container.querySelector<HTMLSelectElement>('select[aria-label="Meeting review export format"]');
    if (!select) throw new Error('Missing export picker');
    await act(async () => {
      select.value = 'vtt';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    expect(container.textContent).toContain('One caption per recorded speech segment');
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Copy captions')?.click());
    expect(meetings.copy).toHaveBeenCalledWith('meeting', 'vtt');
    expect(onNotice).toHaveBeenCalledWith('Meeting captions copied as vtt.');
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Export…')?.click());
    expect(meetings.exportReview).toHaveBeenCalledWith('meeting', 1, 'vtt');
  });

  it('requires capture to stop before caption export', async () => {
    const activeDetail: MeetingDetail = { ...detail, session: { ...detail.session, status: 'active' } };
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ detail: activeDetail })} segments={segments} captureBusy onNotice={() => {}} />));
    const select = container.querySelector<HTMLSelectElement>('select[aria-label="Meeting review export format"]');
    if (!select) throw new Error('Missing export picker');
    await act(async () => {
      select.value = 'srt';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    expect(container.textContent).toContain('Stop this meeting before exporting captions.');
    const buttons = [...container.querySelectorAll('button')].filter((button) => ['Copy captions', 'Export…'].includes(button.textContent ?? ''));
    expect(buttons).toHaveLength(2);
    expect(buttons.every((button) => button.disabled)).toBe(true);
  });

  it('omits playback controls and explains audio retention for transcript-only meetings', async () => {
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller()} segments={segments} captureBusy={false} onNotice={() => {}} />));

    expect(container.querySelector('[aria-label="Meeting audio playback"]')).toBeNull();
    expect(container.textContent).toContain('Audio was not retained for this meeting');
    expect(container.querySelector('[aria-label^="Play segment at"]')).toBeNull();
  });

  it('plays a retained segment from its global offset and canonical channel', async () => {
    const retainedSegments = segments.map((segment) => ({ ...segment, audioAvailable: true }));
    const retainedDetail: MeetingDetail = {
      ...detail,
      session: { ...detail.session, retainAudio: true, durationMs: 4_000 },
      segments: retainedSegments,
    };
    const audio = audioController();
    await act(async () => root.render(
      <MeetingReviewWorkspace
        meetings={controller({ detail: retainedDetail })}
        segments={retainedSegments}
        captureBusy={false}
        meetingAudio={audio}
        onNotice={() => {}}
      />,
    ));

    const themSegment = container.querySelector<HTMLButtonElement>(
      '[aria-label="Play segment at 0:02, Them channel"]',
    );
    await act(async () => themSegment?.click());
    expect(audio.playSegment).toHaveBeenCalledWith({ speaker: 'them', startMs: 2_000 });
    const playAll = [...container.querySelectorAll('button')].find(
      (button) => button.textContent?.trim() === 'Play all',
    );
    await act(async () => playAll?.click());
    expect(audio.playAll).toHaveBeenCalledOnce();

    const channel = container.querySelector<HTMLSelectElement>('[aria-label="Playback channel"]');
    if (!channel) throw new Error('Missing playback channel control');
    await act(async () => {
      channel.value = 'me';
      channel.dispatchEvent(new Event('change', { bubbles: true }));
    });
    expect(audio.setChannel).toHaveBeenCalledWith('me');

    const position = container.querySelector<HTMLInputElement>('[aria-label="Playback position"]');
    expect(position?.getAttribute('aria-valuetext')).toBe('0:00 of 0:04');
    if (!position) throw new Error('Missing playback position control');
    const setRangeValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    if (!setRangeValue) throw new Error('Missing native range setter');
    await act(async () => {
      setRangeValue.call(position, '1500');
      position.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(audio.seek).toHaveBeenCalledWith(1_500);
  });

  it('disables retained-audio playback while another capture path is busy', async () => {
    const retainedSegments = segments.map((segment) => ({ ...segment, audioAvailable: true }));
    const retainedDetail: MeetingDetail = {
      ...detail,
      session: { ...detail.session, retainAudio: true, durationMs: 4_000 },
      segments: retainedSegments,
    };
    await act(async () => root.render(
      <MeetingReviewWorkspace
        meetings={controller({ detail: retainedDetail })}
        segments={retainedSegments}
        captureBusy
        meetingAudio={audioController()}
        onNotice={() => {}}
      />,
    ));

    expect(container.textContent).toContain('Playback is paused while Murmur records or processes a request.');
    expect(container.querySelector<HTMLButtonElement>('[aria-label^="Play segment at"]')?.disabled).toBe(true);
    expect([...container.querySelectorAll<HTMLButtonElement>('[aria-label="Meeting audio playback"] button')]
      .every((button) => button.disabled)).toBe(true);
  });

  it('keeps Pause available while retained audio is buffering', async () => {
    const retainedSegments = segments.map((segment) => ({ ...segment, audioAvailable: true }));
    const retainedDetail: MeetingDetail = {
      ...detail,
      session: { ...detail.session, retainAudio: true, durationMs: 4_000 },
      segments: retainedSegments,
    };
    const pause = vi.fn();
    await act(async () => root.render(
      <MeetingReviewWorkspace
        meetings={controller({ detail: retainedDetail })}
        segments={retainedSegments}
        captureBusy={false}
        meetingAudio={audioController({ status: 'buffering', pause })}
        onNotice={() => {}}
      />,
    ));

    const pauseButton = [...container.querySelectorAll('button')].find(
      (button) => button.textContent?.trim() === 'Pause',
    );
    expect(pauseButton?.disabled).toBe(false);
    await act(async () => pauseButton?.click());
    expect(pause).toHaveBeenCalledOnce();
  });

  it('offers an explicit retry after retained audio playback fails', async () => {
    const retainedDetail: MeetingDetail = {
      ...detail,
      session: { ...detail.session, retainAudio: true, durationMs: 4_000 },
    };
    const retryAudio = vi.fn();
    await act(async () => root.render(
      <MeetingReviewWorkspace
        meetings={controller({ detail: retainedDetail })}
        segments={segments}
        captureBusy={false}
        meetingAudio={audioController({ status: 'error', error: 'Playback safety checks are unavailable.', retry: retryAudio })}
        onNotice={() => {}}
      />,
    ));

    const retry = [...container.querySelectorAll('button')].find(
      (button) => button.textContent?.trim() === 'Retry audio access',
    );
    expect(retry?.disabled).toBe(false);
    await act(async () => retry?.click());
    expect(retryAudio).toHaveBeenCalledOnce();
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

  it('keeps unsaved review and label edits while remote speaker labels refresh', async () => {
    const initialDetail: MeetingDetail = {
      ...detail,
      remoteSpeakers: [
        { speakerId: 1, label: 'Casey' },
        { speakerId: 2, label: 'Riley' },
      ],
    };
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ detail: initialDetail })} segments={segments} captureBusy={false} onNotice={() => {}} />));
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Edit review')!.click());

    const setInputValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    const setTextAreaValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
    const summary = container.querySelector('[aria-label="Review summary"]') as HTMLTextAreaElement;
    const meLabel = container.querySelector('[aria-label="Me speaker label"]') as HTMLInputElement;
    const secondRemote = container.querySelector('[aria-label="Remote speaker 2 label"]') as HTMLInputElement;
    await act(async () => {
      setTextAreaValue.call(summary, 'Unsaved review text');
      summary.dispatchEvent(new Event('input', { bubbles: true }));
      setInputValue.call(meLabel, 'Unsaved me label');
      meLabel.dispatchEvent(new Event('input', { bubbles: true }));
      setInputValue.call(secondRemote, 'Unsaved remote label');
      secondRemote.dispatchEvent(new Event('input', { bubbles: true }));
    });

    const refreshedDetail: MeetingDetail = {
      ...initialDetail,
      remoteSpeakers: [
        { speakerId: 1, label: 'Alex' },
        { speakerId: 2, label: 'Riley' },
      ],
    };
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ detail: refreshedDetail })} segments={segments} captureBusy={false} onNotice={() => {}} />));

    expect((container.querySelector('[aria-label="Review summary"]') as HTMLTextAreaElement).value).toBe('Unsaved review text');
    expect((container.querySelector('[aria-label="Me speaker label"]') as HTMLInputElement).value).toBe('Unsaved me label');
    expect((container.querySelector('[aria-label="Remote speaker 1 label"]') as HTMLInputElement).value).toBe('Alex');
    expect((container.querySelector('[aria-label="Remote speaker 2 label"]') as HTMLInputElement).value).toBe('Unsaved remote label');
  });

  it('keeps edits typed while a remote speaker save completes', async () => {
    let resolveRename: ((saved: boolean) => void) | undefined;
    const renameRemoteSpeaker = vi.fn(() => new Promise<boolean>((resolve) => {
      resolveRename = resolve;
    }));
    const initialDetail: MeetingDetail = {
      ...detail,
      remoteSpeakers: [
        { speakerId: 1, label: 'Casey' },
        { speakerId: 2, label: 'Riley' },
      ],
    };
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ detail: initialDetail, renameRemoteSpeaker })} segments={segments} captureBusy={false} onNotice={() => {}} />));
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Edit review')!.click());

    const setInputValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    const setTextAreaValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
    const summary = container.querySelector('[aria-label="Review summary"]') as HTMLTextAreaElement;
    const meLabel = container.querySelector('[aria-label="Me speaker label"]') as HTMLInputElement;
    const firstRemote = container.querySelector('[aria-label="Remote speaker 1 label"]') as HTMLInputElement;
    const secondRemote = container.querySelector('[aria-label="Remote speaker 2 label"]') as HTMLInputElement;
    await act(async () => {
      setTextAreaValue.call(summary, 'Review typed before save');
      summary.dispatchEvent(new Event('input', { bubbles: true }));
      setInputValue.call(meLabel, 'Me draft');
      meLabel.dispatchEvent(new Event('input', { bubbles: true }));
      setInputValue.call(firstRemote, 'Alex');
      firstRemote.dispatchEvent(new Event('input', { bubbles: true }));
      setInputValue.call(secondRemote, 'Jordan');
      secondRemote.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => (firstRemote.parentElement?.querySelector('button') as HTMLButtonElement).click());

    await act(async () => {
      setInputValue.call(firstRemote, 'Alexandra');
      firstRemote.dispatchEvent(new Event('input', { bubbles: true }));
      setTextAreaValue.call(summary, 'Review typed during save');
      summary.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const savedDetail: MeetingDetail = {
      ...initialDetail,
      remoteSpeakers: [
        { speakerId: 1, label: 'Alex' },
        { speakerId: 2, label: 'Riley' },
      ],
    };
    await act(async () => {
      root.render(<MeetingReviewWorkspace meetings={controller({ detail: savedDetail, renameRemoteSpeaker })} segments={segments} captureBusy={false} onNotice={() => {}} />);
      resolveRename?.(true);
    });

    expect(renameRemoteSpeaker).toHaveBeenCalledWith('meeting', 1, 'Alex');
    expect((container.querySelector('[aria-label="Review summary"]') as HTMLTextAreaElement).value).toBe('Review typed during save');
    expect((container.querySelector('[aria-label="Me speaker label"]') as HTMLInputElement).value).toBe('Me draft');
    expect((container.querySelector('[aria-label="Remote speaker 1 label"]') as HTMLInputElement).value).toBe('Alexandra');
    expect((container.querySelector('[aria-label="Remote speaker 2 label"]') as HTMLInputElement).value).toBe('Jordan');
  });

  it('ignores a remote speaker save completion after switching meetings', async () => {
    let resolveRename: ((saved: boolean) => void) | undefined;
    const renameRemoteSpeaker = vi.fn(() => new Promise<boolean>((resolve) => {
      resolveRename = resolve;
    }));
    const onNotice = vi.fn();
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ renameRemoteSpeaker })} segments={segments} captureBusy={false} onNotice={onNotice} />));
    const firstMeetingInput = container.querySelector('[aria-label="Remote speaker 1 label"]') as HTMLInputElement;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setValue.call(firstMeetingInput, 'Alex');
      firstMeetingInput.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => (firstMeetingInput.parentElement?.querySelector('button') as HTMLButtonElement).click());

    const nextDetail: MeetingDetail = {
      ...detail,
      session: { ...detail.session, id: 'next-meeting' },
      labels: { me: 'Morgan', them: 'Clients' },
      remoteSpeakers: [{ speakerId: 1, label: 'Taylor' }],
    };
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ detail: nextDetail, renameRemoteSpeaker })} segments={segments} captureBusy={false} onNotice={onNotice} />));
    const nextMeetingInput = container.querySelector('[aria-label="Remote speaker 1 label"]') as HTMLInputElement;
    await act(async () => {
      setValue.call(nextMeetingInput, 'Alex');
      nextMeetingInput.dispatchEvent(new Event('input', { bubbles: true }));
      resolveRename?.(true);
    });

    expect((container.querySelector('[aria-label="Remote speaker 1 label"]') as HTMLInputElement).value).toBe('Alex');
    expect(onNotice).not.toHaveBeenCalled();
  });

  it('ignores a channel label save completion after switching meetings', async () => {
    let resolveSave: ((saved: boolean) => void) | undefined;
    const saveReview = vi.fn(() => new Promise<boolean>((resolve) => {
      resolveSave = resolve;
    }));
    const onNotice = vi.fn();
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ saveReview })} segments={segments} captureBusy={false} onNotice={onNotice} />));
    const firstMeetingInput = container.querySelector('[aria-label="Me speaker label"]') as HTMLInputElement;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setValue.call(firstMeetingInput, 'Alex');
      firstMeetingInput.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Save labels')!.click());

    const nextDetail: MeetingDetail = {
      ...detail,
      session: { ...detail.session, id: 'next-meeting' },
      labels: { me: 'Morgan', them: 'Clients' },
    };
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ detail: nextDetail, saveReview })} segments={segments} captureBusy={false} onNotice={onNotice} />));
    const nextMeetingInput = container.querySelector('[aria-label="Me speaker label"]') as HTMLInputElement;
    await act(async () => {
      setValue.call(nextMeetingInput, 'Alex');
      nextMeetingInput.dispatchEvent(new Event('input', { bubbles: true }));
      resolveSave?.(true);
    });

    expect((container.querySelector('[aria-label="Me speaker label"]') as HTMLInputElement).value).toBe('Alex');
    expect(onNotice).not.toHaveBeenCalled();
  });

  it('ignores a save from an earlier activation after switching away and back', async () => {
    let resolveRename: ((saved: boolean) => void) | undefined;
    const renameRemoteSpeaker = vi.fn(() => new Promise<boolean>((resolve) => {
      resolveRename = resolve;
    }));
    const onNotice = vi.fn();
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ renameRemoteSpeaker })} segments={segments} captureBusy={false} onNotice={onNotice} />));
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    const originalInput = container.querySelector('[aria-label="Remote speaker 1 label"]') as HTMLInputElement;
    await act(async () => {
      setValue.call(originalInput, 'Alex');
      originalInput.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => (originalInput.parentElement?.querySelector('button') as HTMLButtonElement).click());

    const otherDetail: MeetingDetail = {
      ...detail,
      session: { ...detail.session, id: 'other-meeting' },
      remoteSpeakers: [{ speakerId: 1, label: 'Taylor' }],
    };
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ detail: otherDetail, renameRemoteSpeaker })} segments={segments} captureBusy={false} onNotice={onNotice} />));
    await act(async () => root.render(<MeetingReviewWorkspace meetings={controller({ renameRemoteSpeaker })} segments={segments} captureBusy={false} onNotice={onNotice} />));
    const reactivatedInput = container.querySelector('[aria-label="Remote speaker 1 label"]') as HTMLInputElement;
    await act(async () => {
      setValue.call(reactivatedInput, 'Alex');
      reactivatedInput.dispatchEvent(new Event('input', { bubbles: true }));
      resolveRename?.(true);
    });

    expect((container.querySelector('[aria-label="Remote speaker 1 label"]') as HTMLInputElement).value).toBe('Alex');
    expect(onNotice).not.toHaveBeenCalled();
  });
});
