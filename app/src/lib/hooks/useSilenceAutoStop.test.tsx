import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useSilenceAutoStop } from './useSilenceAutoStop';
import type { DictationStatus } from '../types';

let levelHandler: ((event: { payload: number }) => void) | null = null;
const unlisten = vi.fn();

vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (e: { payload: number }) => void) => {
    if (event === 'audio-level') levelHandler = handler;
    return Promise.resolve(unlisten);
  },
}));
vi.mock('../log', () => ({ flog: { info: vi.fn(), warn: vi.fn(), error: vi.fn() } }));

function Harness(props: {
  enabled: boolean;
  status: DictationStatus;
  silenceMs: number;
  onAutoStop: () => void;
}) {
  useSilenceAutoStop(props);
  return null;
}

describe('useSilenceAutoStop', () => {
  let container: HTMLDivElement;
  let root: Root;
  let now: number;

  async function render(props: {
    enabled?: boolean;
    status?: DictationStatus;
    silenceMs?: number;
    onAutoStop: () => void;
  }) {
    await act(async () => {
      root.render(
        <Harness
          enabled={props.enabled ?? true}
          status={props.status ?? 'recording'}
          silenceMs={props.silenceMs ?? 2000}
          onAutoStop={props.onAutoStop}
        />,
      );
    });
  }

  /** Push `ms` of audio at `level`, advancing the clock 16ms per sample. */
  async function emit(level: number, ms: number) {
    await act(async () => {
      for (let elapsed = 0; elapsed < ms; elapsed += 16) {
        now += 16;
        levelHandler?.({ payload: level });
      }
    });
  }

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    levelHandler = null;
    unlisten.mockClear();
    now = 1_000_000;
    vi.spyOn(Date, 'now').mockImplementation(() => now);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it('stops once after speech followed by the configured silence', async () => {
    const onAutoStop = vi.fn();
    await render({ onAutoStop });
    await emit(0.3, 800);
    expect(onAutoStop).not.toHaveBeenCalled();
    await emit(0.0, 1900);
    expect(onAutoStop).not.toHaveBeenCalled();
    await emit(0.0, 400);
    expect(onAutoStop).toHaveBeenCalledOnce();
    // Latched: further silence does not fire again.
    await emit(0.0, 5000);
    expect(onAutoStop).toHaveBeenCalledOnce();
  });

  it('never fires when the recording only ever heard silence', async () => {
    const onAutoStop = vi.fn();
    await render({ onAutoStop });
    await emit(0.0, 20_000);
    expect(onAutoStop).not.toHaveBeenCalled();
  });

  it('ignores levels while disabled', async () => {
    const onAutoStop = vi.fn();
    await render({ enabled: false, onAutoStop });
    await emit(0.3, 800);
    await emit(0.0, 5000);
    expect(onAutoStop).not.toHaveBeenCalled();
  });

  it('ignores levels when the setting is off', async () => {
    const onAutoStop = vi.fn();
    await render({ silenceMs: 0, onAutoStop });
    await emit(0.3, 800);
    await emit(0.0, 10_000);
    expect(onAutoStop).not.toHaveBeenCalled();
  });

  it('ignores levels that arrive outside a recording', async () => {
    const onAutoStop = vi.fn();
    await render({ status: 'processing', onAutoStop });
    await emit(0.3, 800);
    await emit(0.0, 5000);
    expect(onAutoStop).not.toHaveBeenCalled();
  });

  it('starts a fresh detector for each recording', async () => {
    const onAutoStop = vi.fn();
    await render({ onAutoStop });
    await emit(0.3, 800);
    await emit(0.0, 2400);
    expect(onAutoStop).toHaveBeenCalledOnce();

    await render({ status: 'idle', onAutoStop });
    await render({ status: 'recording', onAutoStop });
    // A brand-new recording must hear speech again before it can self-stop.
    await emit(0.0, 5000);
    expect(onAutoStop).toHaveBeenCalledOnce();
    await emit(0.3, 800);
    await emit(0.0, 2400);
    expect(onAutoStop).toHaveBeenCalledTimes(2);
  });

  it('unsubscribes on unmount', async () => {
    await render({ onAutoStop: vi.fn() });
    await act(async () => root.unmount());
    expect(unlisten).toHaveBeenCalled();
    root = createRoot(container);
  });
});
