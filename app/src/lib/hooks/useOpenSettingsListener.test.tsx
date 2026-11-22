import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  handler: null as null | ((event: { payload: unknown }) => void),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((_event: string, handler: (event: { payload: unknown }) => void) => {
    mocks.handler = handler;
    return Promise.resolve(() => {});
  }),
}));

import { useOpenSettingsListener, type OpenSettingsDestination } from './useOpenSettingsListener';

function Harness({ onOpen }: { onOpen: (destination?: OpenSettingsDestination) => void }) {
  useOpenSettingsListener(onOpen);
  return null;
}

describe('useOpenSettingsListener', () => {
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

  it('opens the named page for a model cue and General for the bare gear event', async () => {
    const onOpen = vi.fn();
    await act(async () => {
      root.render(<Harness onOpen={onOpen} />);
      await Promise.resolve();
    });
    mocks.handler?.({ payload: { page: 'ai-transcription', target: 'transcription-model' } });
    expect(onOpen).toHaveBeenLastCalledWith({ page: 'ai-transcription', target: 'transcription-model' });
    mocks.handler?.({ payload: undefined });
    expect(onOpen).toHaveBeenLastCalledWith(undefined);
    mocks.handler?.({ payload: { page: 42 } });
    expect(onOpen).toHaveBeenLastCalledWith(undefined);
  });
});
