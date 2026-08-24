import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  transcribeFile: vi.fn(),
  addEntry: vi.fn(),
  unlisten: vi.fn(),
}));

vi.mock('../dictation', () => ({
  transcribeFile: mocks.transcribeFile,
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
    expect(mocks.transcribeFile).toHaveBeenCalledWith('/tmp/selected.wav');
    expect(mocks.addEntry).toHaveBeenCalledWith(
      'transcribed locally',
      1.5,
      'file',
      'selected.wav',
    );
    expect(current.summary).toMatchObject({
      total: 1,
      queued: 0,
      transcribing: 0,
      done: 1,
      error: 0,
      finished: true,
    });
  });
});
