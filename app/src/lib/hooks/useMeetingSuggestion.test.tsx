import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MeetingSuggestion } from '../meetingSuggestions';
import type { MeetingSuggestionController } from './useMeetingSuggestion';

const mocks = vi.hoisted(() => ({
  dismiss: vi.fn(),
  emitTo: vi.fn(),
  get: vi.fn(),
  listener: null as ((event: { payload: unknown }) => void) | null,
}));

vi.mock('@tauri-apps/api/event', () => ({
  emitTo: mocks.emitTo,
  listen: vi.fn(async (_name: string, listener: (event: { payload: unknown }) => void) => {
    mocks.listener = listener;
    return () => { mocks.listener = null; };
  }),
}));
vi.mock('../meetingSuggestions', async (importOriginal) => {
  const original = await importOriginal<typeof import('../meetingSuggestions')>();
  return {
    ...original,
    dismissMeetingSuggestion: mocks.dismiss,
    getMeetingSuggestion: mocks.get,
  };
});

import { useMeetingSuggestion } from './useMeetingSuggestion';

const first: MeetingSuggestion = {
  token: '11111111-1111-4111-8111-111111111111',
  title: 'Product review',
  startMs: 1,
  endMs: 2,
};
const second: MeetingSuggestion = {
  token: '22222222-2222-4222-8222-222222222222',
  title: 'Design review',
  startMs: 3,
  endMs: 4,
};

function deferred<T>() {
  let resolve = (_value: T) => {};
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe('useMeetingSuggestion', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: MeetingSuggestionController;

  function Harness() {
    current = useMeetingSuggestion();
    return null;
  }

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listener = null;
    mocks.get.mockResolvedValue(null);
    mocks.dismiss.mockResolvedValue(undefined);
    mocks.emitTo.mockResolvedValue(undefined);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  async function mount() {
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  it('subscribes before fetching and drops a stale initial snapshot behind a newer event', async () => {
    const snapshot = deferred<MeetingSuggestion | null>();
    mocks.get.mockReturnValue(snapshot.promise);
    await mount();

    await act(async () => mocks.listener?.({ payload: second }));
    await act(async () => snapshot.resolve(first));

    expect(current.suggestion).toEqual(second);
  });

  it('emits one token-only acceptance while a double click is in flight', async () => {
    const emission = deferred<void>();
    mocks.emitTo.mockReturnValue(emission.promise);
    await mount();
    await act(async () => mocks.listener?.({ payload: first }));

    void current.accept();
    void current.accept();
    expect(mocks.emitTo).toHaveBeenCalledTimes(1);
    expect(mocks.emitTo).toHaveBeenCalledWith(
      'main',
      'accept-meeting-suggestion',
      { token: first.token },
    );
    expect(JSON.stringify(mocks.emitTo.mock.calls)).not.toContain(first.title);

    await act(async () => emission.resolve(undefined));
    expect(current.suggestion).toBeNull();
  });

  it('dismisses by token and clears promptly when the backend turns suggestions off', async () => {
    await mount();
    await act(async () => mocks.listener?.({ payload: first }));
    await act(async () => current.dismiss());
    expect(mocks.dismiss).toHaveBeenCalledWith(first.token);
    expect(current.suggestion).toBeNull();

    await act(async () => mocks.listener?.({ payload: second }));
    expect(current.suggestion).toEqual(second);
    await act(async () => mocks.listener?.({ payload: null }));
    expect(current.suggestion).toBeNull();
  });

  it('rejects unbounded or malformed native prompt payloads', async () => {
    await mount();
    await act(async () => mocks.listener?.({
      payload: { ...first, title: 'x'.repeat(201) },
    }));
    expect(current.suggestion).toBeNull();

    await act(async () => mocks.listener?.({
      payload: { ...first, token: 'not-a-uuid' },
    }));
    expect(current.suggestion).toBeNull();

    await act(async () => mocks.listener?.({
      payload: { ...first, endMs: first.startMs },
    }));
    expect(current.suggestion).toBeNull();

    const astralBoundary = { ...first, title: '😀'.repeat(200) };
    await act(async () => mocks.listener?.({ payload: astralBoundary }));
    expect(current.suggestion).toEqual(astralBoundary);
  });
});
