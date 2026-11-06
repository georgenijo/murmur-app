import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  transcribeFile: vi.fn(),
  cancelFileTranscription: vi.fn().mockResolvedValue(undefined),
  acceptFileTranscription: vi.fn().mockResolvedValue(undefined),
  addEntry: vi.fn(),
  unlisten: vi.fn(),
}));

vi.mock('../dictation', () => ({
  transcribeFile: mocks.transcribeFile,
  cancelFileTranscription: mocks.cancelFileTranscription,
  acceptFileTranscription: mocks.acceptFileTranscription,
}));

vi.mock('../log', () => ({
  flog: {
    info: vi.fn(),
    warn: vi.fn(),
  },
}));

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: vi.fn(async () => mocks.unlisten),
  }),
}));

import { useFileTranscription } from './useFileTranscription';

describe('useFileTranscription', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: ReturnType<typeof useFileTranscription>;

  function Harness() {
    current = useFileTranscription({ addEntry: mocks.addEntry });
    return null;
  }

  beforeEach(() => {
    mocks.transcribeFile.mockReset().mockResolvedValue({
      type: 'success',
      text: 'transcribed locally',
      duration: 1.5,
    });
    mocks.addEntry.mockReset();
    mocks.acceptFileTranscription.mockClear();
    mocks.cancelFileTranscription.mockClear();
    mocks.unlisten.mockReset();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('starts draining a file selected into an empty queue', async () => {
    await act(async () => root.render(<Harness />));

    await act(async () => {
      current.enqueue(['/tmp/selected.wav']);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(mocks.transcribeFile).toHaveBeenCalledOnce();
    expect(mocks.transcribeFile).toHaveBeenCalledWith('/tmp/selected.wav', expect.any(String));
    expect(mocks.addEntry).not.toHaveBeenCalled();
    expect(current.summary).toMatchObject({
      total: 1,
      queued: 0,
      transcribing: 0,
      done: 1,
      error: 0,
      finished: true,
    });
  });
  it('sends cancellation with the operation identity and never accepts a cancelled result', async () => {
    let finish!: (value: unknown) => void;
    mocks.transcribeFile.mockReturnValue(new Promise(resolve => { finish = resolve; }));
    await act(async () => root.render(<Harness />));
    await act(async () => current.enqueue(['/tmp/cancelled.wav']));
    const id = current.queue[0].id;
    await act(async () => current.cancel(id));
    expect(mocks.cancelFileTranscription).toHaveBeenCalledWith(mocks.transcribeFile.mock.calls[0][1]);
    await act(async () => finish({ type: 'file_transcription', text: 'synthetic result', duration: 2 }));
    expect(current.queue[0]).toMatchObject({ status: 'error', error: 'Canceled' });
    expect(mocks.acceptFileTranscription).not.toHaveBeenCalled();
  });

  it('accepts only the operation whose result is visible; cancel after completion is inert', async () => {
    await act(async () => root.render(<Harness />));
    await act(async () => current.enqueue(['/tmp/completed.wav']));
    const id = current.queue[0].id;
    expect(mocks.acceptFileTranscription).toHaveBeenCalledWith(mocks.transcribeFile.mock.calls[0][1]);
    await act(async () => current.cancel(id));
    expect(current.queue[0].status).toBe('done');
    expect(mocks.cancelFileTranscription).not.toHaveBeenCalled();
  });

  it('gives repeated imports of the same path distinct native operation identities', async () => {
    await act(async () => root.render(<Harness />));
    await act(async () => current.enqueue(['/tmp/repeated.wav']));
    const first = mocks.transcribeFile.mock.calls[0][1];
    await act(async () => current.reset());
    await act(async () => current.enqueue(['/tmp/repeated.wav']));
    expect(mocks.transcribeFile.mock.calls[1][1]).not.toBe(first);
    expect(mocks.acceptFileTranscription).toHaveBeenCalledTimes(2);
  });

});
