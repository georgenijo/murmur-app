import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MeetingDiarizationSettings } from './MeetingDiarizationSettings';

const meetingMocks = vi.hoisted(() => ({
  getStatus: vi.fn(),
  download: vi.fn(),
  remove: vi.fn(),
}));
const eventMocks = vi.hoisted(() => ({
  listener: null as ((event: { payload: unknown }) => void) | null,
}));

vi.mock('../../lib/meetings', () => ({
  MEETING_DIARIZATION_MODEL_ID: 'meeting-diarization-coreml',
  getDiarizationModelStatus: meetingMocks.getStatus,
  downloadDiarizationModel: meetingMocks.download,
  removeDiarizationModel: meetingMocks.remove,
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_name: string, listener: (event: { payload: unknown }) => void) => {
    eventMocks.listener = listener;
    return () => {
      eventMocks.listener = null;
    };
  }),
}));

describe('MeetingDiarizationSettings', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    meetingMocks.getStatus.mockReset();
    meetingMocks.download.mockReset();
    meetingMocks.remove.mockReset();
    meetingMocks.download.mockResolvedValue(undefined);
    meetingMocks.remove.mockResolvedValue(undefined);
    eventMocks.listener = null;
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('keeps the opt-in explicit and discloses temporary audio and fallback behavior', async () => {
    meetingMocks.getStatus.mockResolvedValue({ supported: true, installed: false, installing: false, bytes: 21_599_417 });
    const onEnabledChange = vi.fn();
    await act(async () => root.render(
      <MeetingDiarizationSettings enabled={false} onEnabledChange={onEnabledChange} />,
    ));

    const toggle = container.querySelector('[role="switch"][aria-label="Label remote speakers"]') as HTMLButtonElement;
    expect(toggle.getAttribute('aria-checked')).toBe('false');
    expect(container.textContent).toContain('saves up to two hours of system audio');
    expect(container.textContent).toContain('speaker labeling is skipped while capture and transcription continue');
    expect(container.textContent).toContain('Uncertain passages remain Them');
    expect(container.textContent).toContain('does not create voice profiles across meetings');
    expect(container.querySelector('a[href*="FluidInference/speaker-diarization-coreml"]')).toBeTruthy();
    expect(container.querySelector('a[href="https://creativecommons.org/licenses/by/4.0/"]')).toBeTruthy();
    await act(async () => toggle.click());
    expect(onEnabledChange).toHaveBeenCalledWith(true);
  });

  it('folds the model, disclosure and status errors with its parent', async () => {
    meetingMocks.getStatus.mockRejectedValue(new Error('Model status unavailable'));
    const onEnabledChange = vi.fn();
    const render = (enabled: boolean) => act(async () => root.render(
      <MeetingDiarizationSettings enabled={enabled} onEnabledChange={onEnabledChange} />,
    ));
    await render(false);
    const branch = container.querySelector('.settings-dependent-branch')!;
    expect(branch.getAttribute('aria-hidden')).toBe('true');
    expect(branch.hasAttribute('inert')).toBe(true);
    expect(branch.textContent).toContain('Remote speaker model');
    expect(branch.textContent).toContain('Labels stay on this Mac');
    expect(branch.querySelector('[role="alert"]')?.textContent).toContain('Model status unavailable');
    await render(true);
    expect(branch.getAttribute('aria-hidden')).toBe('false');
    expect(branch.hasAttribute('inert')).toBe(false);
    await render(false);
    expect(branch.getAttribute('aria-hidden')).toBe('true');
    expect(branch.hasAttribute('inert')).toBe(true);
  });

  it.each([true, false])('resets remote-speaker labeling through its owner and respects supported=%s', async (supported) => {
    meetingMocks.getStatus.mockResolvedValue({ supported, installed: false, installing: false, bytes: 21_599_417 });
    const onEnabledChange = vi.fn();
    await act(async () => root.render(<MeetingDiarizationSettings enabled={false} onEnabledChange={onEnabledChange} />));
    expect(container.querySelector('[aria-label="Reset to default"]')).toBeNull();
    await act(async () => root.render(<MeetingDiarizationSettings enabled onEnabledChange={onEnabledChange} />));
    const reset = container.querySelector<HTMLButtonElement>('[aria-label="Reset to default"]')!;
    if (!supported) {
      expect(reset).toBeNull();
      expect(container.textContent).toBe('');
      return;
    }
    expect(reset.disabled).toBe(false);
    await act(async () => reset.click());
    if (supported) expect(onEnabledChange).toHaveBeenCalledExactlyOnceWith(false);
    else expect(onEnabledChange).not.toHaveBeenCalled();
  });

  it('downloads with correlated progress and can remove the installed model', async () => {
    meetingMocks.getStatus
      .mockResolvedValueOnce({ supported: true, installed: false, installing: false, bytes: 21_599_417 })
      .mockResolvedValueOnce({ supported: true, installed: true, installing: false, bytes: 21_599_417 })
      .mockResolvedValueOnce({ supported: true, installed: false, installing: false, bytes: 21_599_417 });
    let finishDownload: (() => void) | null = null;
    meetingMocks.download.mockImplementation(() => new Promise<void>((resolve) => {
      finishDownload = resolve;
    }));
    await act(async () => root.render(
      <MeetingDiarizationSettings enabled onEnabledChange={() => {}} />,
    ));

    const download = Array.from(container.querySelectorAll('button')).find((button) => button.textContent === 'Download model') as HTMLButtonElement;
    await act(async () => download.click());
    await act(async () => eventMocks.listener?.({ payload: {
      modelName: 'meeting-diarization-coreml', attemptId: 7,
      received: 10_000_000, total: 20_000_000, phase: 'downloading',
    } }));
    expect(container.textContent).toContain('Downloading… 50%');
    await act(async () => root.render(<MeetingDiarizationSettings enabled={false} onEnabledChange={() => {}} />));
    const progressBranch = container.querySelector('[role="status"]')!.closest('.settings-dependent-branch')!;
    expect(progressBranch.getAttribute('aria-hidden')).toBe('true');
    expect(progressBranch.hasAttribute('inert')).toBe(true);
    await act(async () => root.render(<MeetingDiarizationSettings enabled onEnabledChange={() => {}} />));
    expect(progressBranch.getAttribute('aria-hidden')).toBe('false');
    await act(async () => finishDownload?.());

    expect(meetingMocks.download).toHaveBeenCalledOnce();
    expect(container.textContent).toContain('Installed locally');
    const remove = Array.from(container.querySelectorAll('button')).find((button) => button.textContent === 'Remove model…') as HTMLButtonElement;
    await act(async () => remove.click());
    expect(meetingMocks.remove).not.toHaveBeenCalled();
    await act(async () => remove.click());
    expect(meetingMocks.remove).toHaveBeenCalledOnce();
    expect(container.textContent).toContain('Optional download');
  });
});
