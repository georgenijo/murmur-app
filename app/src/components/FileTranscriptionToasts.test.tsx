import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FileTranscriptionToasts } from './FileTranscriptionToasts';
import type { QueueItem } from '../lib/fileQueue';

function item(status: QueueItem['status']): QueueItem {
  return {
    id: `job-${status}`,
    path: `/tmp/${status}.wav`,
    name: `${status}.wav`,
    status,
  };
}

describe('FileTranscriptionToasts', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.useFakeTimers();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it('offers cancellation only for the in-flight file', async () => {
    const onCancel = vi.fn();
    await act(async () => root.render(
      <FileTranscriptionToasts
        queue={[item('queued'), item('transcribing')]}
        error=""
        onCancel={onCancel}
        onDismiss={vi.fn()}
      />,
    ));

    const cancel = Array.from(container.querySelectorAll('button')).find((button) => button.textContent === 'Cancel') as HTMLButtonElement;
    await act(async () => cancel.click());
    expect(onCancel).toHaveBeenCalledWith('job-transcribing');
    expect(container.textContent).toContain('Queued');
  });

  it('auto-dismisses completed jobs after three seconds', async () => {
    const onDismiss = vi.fn();
    await act(async () => root.render(
      <FileTranscriptionToasts
        queue={[item('done')]}
        error=""
        onCancel={vi.fn()}
        onDismiss={onDismiss}
      />,
    ));

    await act(async () => vi.advanceTimersByTime(2999));
    expect(onDismiss).not.toHaveBeenCalled();
    await act(async () => vi.advanceTimersByTime(1));
    expect(onDismiss).toHaveBeenCalledWith('job-done');
  });
});
