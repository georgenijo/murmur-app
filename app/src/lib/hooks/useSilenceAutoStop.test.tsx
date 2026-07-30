import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useSilenceAutoStop } from './useSilenceAutoStop';
import type { DictationStatus } from '../types';

let levelHandler: ((event: { payload: number }) => void) | null = null;
const originHandlers = new Map<string, () => void>();
const unlisten = vi.fn();

vi.mock('@tauri-apps/api/event', () => ({
  listen: (event: string, handler: (e: { payload: number }) => void) => {
    if (event === 'audio-level') levelHandler = handler;
    else originHandlers.set(event, () => handler({ payload: 0 } as never));
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

  /** Fire one of the keyboard-origin events (hold-down-start, …). */
  async function emitOrigin(event: string) {
    await act(async () => {
      originHandlers.get(event)?.();
    });
  }

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    levelHandler = null;
    originHandlers.clear();
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

  it('never fires for a hold-started recording', async () => {
    const onAutoStop = vi.fn();
    await render({ onAutoStop });
    await emitOrigin('hold-down-start');
    await emit(0.3, 800);
    await emit(0.0, 10_000);
    expect(onAutoStop).not.toHaveBeenCalled();
  });

  it('re-arms for the next toggle recording after a hold ends', async () => {
    const onAutoStop = vi.fn();
    await render({ onAutoStop });
    await emitOrigin('hold-down-start');
    await emit(0.3, 800);
    await emit(0.0, 5000);
    expect(onAutoStop).not.toHaveBeenCalled();
    await emitOrigin('hold-down-stop');

    await render({ status: 'idle', onAutoStop });
    await render({ status: 'recording', onAutoStop });
    await emit(0.3, 800);
    await emit(0.0, 2400);
    expect(onAutoStop).toHaveBeenCalledOnce();
  });

  it('does not stay disarmed after a cancelled speculative hold (legacy hold-down-cancel)', async () => {
    const onAutoStop = vi.fn();
    await render({ onAutoStop });
    // hold-down-cancel is a legacy event the current backend never emits
    // (deferred-hold: short taps in Both mode emit nothing) — handled
    // defensively so a start-then-cancel sequence can never strand the origin.
    await emitOrigin('hold-down-start');
    await emitOrigin('hold-down-cancel');
    // The follow-up double-tap starts a fresh toggle recording.
    await render({ status: 'idle', onAutoStop });
    await render({ status: 'recording', onAutoStop });
    await emit(0.3, 800);
    await emit(0.0, 2400);
    expect(onAutoStop).toHaveBeenCalledOnce();
  });

  it('heals a hold origin stranded without its stop event once the recording ends', async () => {
    const onAutoStop = vi.fn();
    await render({ onAutoStop });
    // Escape cancels a hold recording and suppresses the release's
    // hold-down-stop, so no keyboard event ever resets the origin. The
    // recording ending (status leaving 'recording') must heal it, or every
    // later button/overlay recording silently loses auto-stop.
    await emitOrigin('hold-down-start');
    await emit(0.3, 800);
    await render({ status: 'idle', onAutoStop });
    await render({ status: 'recording', onAutoStop });
    await emit(0.3, 800);
    await emit(0.0, 2400);
    expect(onAutoStop).toHaveBeenCalledOnce();
  });

  it('heals a hold origin when initialization is cancelled before readiness', async () => {
    const onAutoStop = vi.fn();
    await render({ status: 'idle', onAutoStop });
    await emitOrigin('hold-down-start');
    await render({ status: 'starting', onAutoStop });
    await render({ status: 'recovering', onAutoStop });
    await render({ status: 'idle', onAutoStop });
    await render({ status: 'recording', onAutoStop });
    await emit(0.3, 800);
    await emit(0.0, 2400);
    expect(onAutoStop).toHaveBeenCalledOnce();
  });

  it('treats a double-tap after a stale hold origin as toggle-started', async () => {
    const onAutoStop = vi.fn();
    await render({ onAutoStop });
    await emitOrigin('hold-down-start');
    await emitOrigin('double-tap-toggle');
    await emit(0.3, 800);
    await emit(0.0, 2400);
    expect(onAutoStop).toHaveBeenCalledOnce();
  });

  it('unsubscribes on unmount', async () => {
    await render({ onAutoStop: vi.fn() });
    await act(async () => root.unmount());
    expect(unlisten).toHaveBeenCalled();
    root = createRoot(container);
  });
});
