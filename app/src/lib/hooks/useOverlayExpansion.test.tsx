import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  COLLAPSE_DELAY_MS,
  HOVER_OPEN_DWELL_MS,
  OVERLAY_HEIGHT_MS,
  SHRINK_DELAY_MS,
} from '../overlayMotion';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  cursorPosition: vi.fn(),
  outerPosition: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ outerPosition: mocks.outerPosition }),
  cursorPosition: mocks.cursorPosition,
}));
vi.mock('../log', () => ({
  flog: { info: vi.fn(), warn: vi.fn(), error: vi.fn() },
}));

import { useOverlayExpansion, type OverlayExpansion } from './useOverlayExpansion';

// A pending set_overlay_expanded invocation the test controls.
interface SurfaceCall {
  args: { expanded: boolean };
  resolve: (v: { windowW: number; windowH: number }) => void;
  reject: (e: unknown) => void;
}

const APPLIED = { windowW: 305, windowH: 76 };

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe('useOverlayExpansion', () => {
  let container: HTMLDivElement | null = null;
  let root: Root | null = null;
  let current: OverlayExpansion;
  let surfaceCalls: SurfaceCall[];
  let lastProps: { disabled?: boolean; withIsland?: boolean } = {};
  const listeners = new Map<string, (e: { payload: unknown }) => void>();

  function Harness(props: { disabled?: boolean; withIsland?: boolean }) {
    current = useOverlayExpansion({
      disabled: props.disabled ?? false,
    });
    return props.withIsland ? <div ref={current.islandRef} /> : null;
  }

  async function rerender(props: { disabled?: boolean; withIsland?: boolean }) {
    lastProps = { ...lastProps, ...props };
    await act(async () => { root!.render(<Harness {...lastProps} />); });
  }

  function emitEvent(event: string, payload: unknown) {
    listeners.get(event)?.({ payload });
  }

  function surfaceCallCount() {
    return mocks.invoke.mock.calls.filter((c) => c[0] === 'set_overlay_expanded').length;
  }

  async function flush() {
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
  }

  async function mount(props: { disabled?: boolean; withIsland?: boolean } = {}) {
    // mount owns the root's whole lifecycle so beforeEach never leaks an empty
    // container. A prior mount (a test that re-mounts) is torn down first.
    if (root) { await act(async () => { root!.unmount(); }); }
    if (container) { container.remove(); }
    lastProps = props;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => { root!.render(<Harness {...props} />); });
  }

  beforeEach(() => {
    vi.useFakeTimers();
    surfaceCalls = [];
    listeners.clear();

    mocks.invoke.mockReset();
    mocks.invoke.mockImplementation((cmd: string, args: unknown) => {
      if (cmd === 'set_overlay_expanded') {
        return new Promise((resolve, reject) => {
          surfaceCalls.push({ args: args as SurfaceCall['args'], resolve, reject });
        });
      }
      return Promise.resolve();
    });

    mocks.listen.mockReset();
    mocks.listen.mockImplementation(async (event: string, handler: (e: { payload: unknown }) => void) => {
      listeners.set(event, handler);
      return () => listeners.delete(event);
    });

    mocks.cursorPosition.mockReset();
    mocks.cursorPosition.mockResolvedValue({ x: -9999, y: -9999 });
    mocks.outerPosition.mockReset();
    mocks.outerPosition.mockResolvedValue({ x: 0, y: 0 });
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn().mockReturnValue({
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    });

    root = null;
    container = null;
    lastProps = {};
  });

  afterEach(async () => {
    if (root) { await act(async () => { root!.unmount(); }); root = null; }
    if (container) { container.remove(); container = null; }
    vi.useRealTimers();
  });

  it('reveals the card only after the grow resize is acknowledged (dwell → opening → ack → open)', async () => {
    await mount();

    await act(async () => { current.onHoverStart(); });
    // Dwell has not elapsed yet — still collapsed.
    expect(current.phase).toBe('collapsed');

    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    // Opening: the resize is enqueued but the card is NOT revealed yet.
    expect(current.phase).toBe('opening');
    expect(current.expanded).toBe(false);
    const grow = surfaceCalls.find((c) => c.args.expanded);
    expect(grow).toBeTruthy();

    await act(async () => { grow!.resolve(APPLIED); });
    await flush();
    // Ack landed — now the card reveals.
    expect(current.phase).toBe('open');
    expect(current.expanded).toBe(true);
  });

  it('leaves → closing → enqueues the shrink after the close animation → collapsed', async () => {
    await mount();
    // Open fully.
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    await act(async () => { surfaceCalls.find((c) => c.args.expanded)!.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('open');
    surfaceCalls.length = 0;

    await act(async () => { current.onHoverEnd(); });
    // Still open during the leave-delay.
    expect(current.phase).toBe('open');

    await act(async () => { vi.advanceTimersByTime(COLLAPSE_DELAY_MS); });
    // Closing: dropdown hidden immediately, but the shrink is not enqueued yet.
    expect(current.phase).toBe('closing');
    expect(current.expanded).toBe(false);
    expect(surfaceCalls.some((c) => !c.args.expanded)).toBe(false);

    await act(async () => { vi.advanceTimersByTime(SHRINK_DELAY_MS); });
    await flush();
    const shrink = surfaceCalls.find((c) => !c.args.expanded);
    expect(shrink).toBeTruthy();

    await act(async () => { shrink!.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('collapsed');
  });

  it('rapid enter/leave/enter applies no stale resize, ends open, and ignores the older-generation ack', async () => {
    await mount();

    // Enter → opening (gen A grow, left pending).
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    expect(current.phase).toBe('opening');
    const growA = surfaceCalls.find((c) => c.args.expanded);
    expect(growA).toBeTruthy();

    // Leave → closing → shrink fires (gen B collapse), but it is queued behind the
    // still-in-flight grow and never actually invokes.
    await act(async () => { current.onHoverEnd(); });
    await act(async () => { vi.advanceTimersByTime(COLLAPSE_DELAY_MS); });
    expect(current.phase).toBe('closing');
    await act(async () => { vi.advanceTimersByTime(SHRINK_DELAY_MS); });
    await flush();

    // Re-enter → opening again (gen C grow), also queued behind gen A.
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    expect(current.phase).toBe('opening');

    // Resolve the ORIGINAL (now stale) grow ack. It must not reveal, and the
    // superseded collapse must never resize. Draining the chain runs the newest
    // grow, which is the only other actual resize.
    await act(async () => { growA!.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('opening'); // stale ack ignored — no premature reveal

    // No collapse resize was ever applied (the superseded shrink was skipped).
    expect(surfaceCalls.some((c) => !c.args.expanded)).toBe(false);

    // The newest grow is now in flight — resolving it reveals.
    const grows = surfaceCalls.filter((c) => c.args.expanded);
    const growC = grows[grows.length - 1];
    await act(async () => { growC.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('open');
  });

  it('leave during opening immediately supersedes the grow and never reveals from its stale ack', async () => {
    await mount();

    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    expect(current.phase).toBe('opening');
    const grow = surfaceCalls.find((c) => c.args.expanded)!;

    await act(async () => { current.onHoverEnd(); });
    expect(current.phase).toBe('closing');
    expect(current.expanded).toBe(false);

    await act(async () => { grow.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('closing');
    expect(current.expanded).toBe(false);

    const collapse = surfaceCalls.find((c) => !c.args.expanded)!;
    await act(async () => { collapse.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('collapsed');
  });

  it('re-entry while closing reopens cleanly and cancels the pending shrink', async () => {
    await mount();
    // Open fully.
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    await act(async () => { surfaceCalls.find((c) => c.args.expanded)!.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('open');

    // Leave → advance into closing.
    await act(async () => { current.onHoverEnd(); });
    await act(async () => { vi.advanceTimersByTime(COLLAPSE_DELAY_MS); });
    expect(current.phase).toBe('closing');
    surfaceCalls.length = 0;

    // Re-enter while closing: cancels shrink, arms dwell, reopens.
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    expect(current.phase).toBe('opening');
    await act(async () => { surfaceCalls.find((c) => c.args.expanded)!.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('open');

    // The cancelled shrink must never have resized.
    expect(surfaceCalls.some((c) => !c.args.expanded)).toBe(false);
  });

  it('reverts to collapsed without revealing when the grow resize is rejected', async () => {
    await mount();

    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    expect(current.phase).toBe('opening');
    const grow = surfaceCalls.find((c) => c.args.expanded);

    await act(async () => { grow!.reject(new Error('resize failed')); });
    await flush();
    expect(current.phase).toBe('collapsed');
    expect(current.expanded).toBe(false);
  });

  it('times out a hung grow so the serialized writer can process a later request', async () => {
    await mount();

    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    const hungGrow = surfaceCalls.find((c) => c.args.expanded)!;
    expect(current.phase).toBe('opening');

    await act(async () => { vi.advanceTimersByTime(2_000); });
    await flush();
    expect(current.phase).toBe('collapsed');
    expect(current.expanded).toBe(false);

    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    const grows = surfaceCalls.filter((c) => c.args.expanded);
    expect(grows).toHaveLength(2);
    await act(async () => { grows[1].resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('open');

    // The timed-out request can settle later, but its wrapper is already rejected
    // and therefore cannot reconcile over the newer successful generation.
    await act(async () => { hungGrow.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('open');
  });

  it('display change mid-open cancels timers, forces collapsed, and issues one corrective collapse', async () => {
    await mount();
    // Open fully.
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    await act(async () => { surfaceCalls.find((c) => c.args.expanded)!.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('open');
    surfaceCalls.length = 0;

    // Arm a close, then a display change arrives.
    await act(async () => { current.onHoverEnd(); });
    await act(async () => { emitEvent('overlay-geometry-changed', {}); });
    await flush();
    expect(current.phase).toBe('collapsed');

    // Exactly one corrective collapse resize is enqueued — it supersedes any
    // straggler grow that could re-grow the window after Rust's reposition.
    const collapseWrites = surfaceCalls.filter((c) => !c.args.expanded);
    expect(collapseWrites.length).toBe(1);
    await act(async () => { collapseWrites[0].resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('collapsed');

    // Timers were cancelled — advancing does not resurrect closing/shrink.
    const before = surfaceCallCount();
    await act(async () => { vi.advanceTimersByTime(SHRINK_DELAY_MS + COLLAPSE_DELAY_MS); });
    await flush();
    expect(current.phase).toBe('collapsed');
    expect(surfaceCallCount()).toBe(before);
  });

  it('clears timers on unmount and issues no further resizes', async () => {
    await mount();
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    expect(current.phase).toBe('opening');
    const pending = surfaceCalls.find((c) => c.args.expanded);
    const before = surfaceCallCount();

    await act(async () => { root!.unmount(); });
    root = null;

    // Nothing new should invoke after unmount, even as timers advance and the
    // in-flight ack resolves.
    await act(async () => { vi.advanceTimersByTime(SHRINK_DELAY_MS * 4); });
    await act(async () => { pending!.resolve(APPLIED); await Promise.resolve(); });
    expect(surfaceCallCount()).toBe(before);
  });

  it('poller performs no IPC while the overlay is hidden or the app is disabled (collapsed)', async () => {
    // Disabled while collapsed → the entry detector is gated for battery.
    await mount({ withIsland: true, disabled: true });
    mocks.cursorPosition.mockClear();
    mocks.outerPosition.mockClear();
    const surfacesBefore = surfaceCallCount();
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS * 4); });
    await flush();
    expect(mocks.cursorPosition).not.toHaveBeenCalled();
    expect(mocks.outerPosition).not.toHaveBeenCalled();
    expect(surfaceCallCount()).toBe(surfacesBefore);

    // Enabled but hidden → still fully gated regardless of phase.
    await rerender({ disabled: false });
    await act(async () => { emitEvent('overlay-visible-changed', false); });
    mocks.cursorPosition.mockClear();
    mocks.outerPosition.mockClear();
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS * 4); });
    await flush();
    expect(mocks.cursorPosition).not.toHaveBeenCalled();
    expect(mocks.outerPosition).not.toHaveBeenCalled();

    // Prove the gate is what stops it: visible + enabled → the poller does IPC.
    await act(async () => { emitEvent('overlay-visible-changed', true); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    expect(mocks.cursorPosition).toHaveBeenCalled();
  });

  it('blocks DOM hover entry while disabled as well as poller entry', async () => {
    await mount({ disabled: true });
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS * 2); });
    await flush();
    expect(current.phase).toBe('collapsed');
    expect(surfaceCallCount()).toBe(0);
  });

  it('drops a cursor result that resolves after the overlay becomes hidden', async () => {
    const outer = deferred<{ x: number; y: number }>();
    const cursor = deferred<{ x: number; y: number }>();
    mocks.outerPosition.mockReturnValueOnce(outer.promise);
    mocks.cursorPosition.mockReturnValueOnce(cursor.promise);
    await mount({ withIsland: true });

    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await act(async () => { emitEvent('overlay-visible-changed', false); });
    await flush();
    const collapse = surfaceCalls.find((c) => !c.args.expanded)!;
    await act(async () => { collapse.resolve(APPLIED); });

    await act(async () => {
      outer.resolve({ x: 0, y: 0 });
      cursor.resolve({ x: 0, y: 0 });
      await Promise.all([outer.promise, cursor.promise]);
    });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS * 2); });
    await flush();
    expect(current.phase).toBe('collapsed');
    expect(surfaceCalls.some((c) => c.args.expanded)).toBe(false);
  });

  it('keeps the exit watchdog alive while open even when the app is disabled', async () => {
    // Cursor inside the (zero-sized jsdom) island bounds so the poller never
    // closes the card while we drive it open.
    mocks.cursorPosition.mockResolvedValue({ x: 0, y: 0 });
    await mount({ withIsland: true, disabled: false });

    // Open fully.
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    await act(async () => { surfaceCalls.find((c) => c.args.expanded)!.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('open');

    // Disable the app while the dropdown is open (user clicks the Disable control),
    // then a DOM mouseleave is missed and the cursor moves outside the card.
    await rerender({ disabled: true });
    mocks.cursorPosition.mockResolvedValue({ x: 9999, y: 9999 });

    // The exit watchdog must still run and collapse the card despite `disabled`.
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    await act(async () => { vi.advanceTimersByTime(1); }); // fire the immediate close timer
    expect(current.phase).toBe('closing');
  });

  it('retries a rejected shrink in the serialized writer before settling collapsed', async () => {
    await mount();
    // Open fully.
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    await act(async () => { surfaceCalls.find((c) => c.args.expanded)!.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('open');
    surfaceCalls.length = 0;

    // Leave → closing → shrink enqueued.
    await act(async () => { current.onHoverEnd(); });
    await act(async () => { vi.advanceTimersByTime(COLLAPSE_DELAY_MS); });
    expect(current.phase).toBe('closing');
    await act(async () => { vi.advanceTimersByTime(SHRINK_DELAY_MS); });
    await flush();
    const shrink = surfaceCalls.find((c) => !c.args.expanded);
    expect(shrink).toBeTruthy();

    // A transient native failure must keep the controller in closing and retry the
    // collapse rather than declaring success with a tall transparent window.
    await act(async () => { shrink!.reject(new Error('resize failed')); });
    await flush();
    expect(current.phase).toBe('closing');
    expect(surfaceCalls.filter((c) => !c.args.expanded)).toHaveLength(1);

    await act(async () => { vi.advanceTimersByTime(100); });
    await flush();
    const retry = surfaceCalls.filter((c) => !c.args.expanded)[1];
    expect(retry).toBeTruthy();

    await act(async () => { retry.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('collapsed');
    expect(current.expanded).toBe(false);
  });

  it('serializes a collapse retry ahead of a re-entry grow without racing the frame', async () => {
    await mount();
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    await act(async () => { surfaceCalls.find((c) => c.args.expanded)!.resolve(APPLIED); });
    await flush();

    await act(async () => { current.onHoverEnd(); });
    await act(async () => { vi.advanceTimersByTime(COLLAPSE_DELAY_MS + SHRINK_DELAY_MS); });
    await flush();
    const shrink = surfaceCalls.find((c) => !c.args.expanded)!;
    await act(async () => { shrink.reject(new Error('resize failed')); });
    await flush();
    expect(current.phase).toBe('closing');

    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(100); });
    await flush();
    const retry = surfaceCalls.filter((c) => !c.args.expanded)[1];
    expect(retry).toBeTruthy();
    expect(current.phase).toBe('closing');

    // The dwell completes while the retry is in flight. The next grow is queued,
    // not dispatched concurrently, until the collapse retry acknowledges.
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS - 100); });
    await flush();
    expect(current.phase).toBe('opening');
    expect(surfaceCalls.filter((c) => c.args.expanded)).toHaveLength(1);

    await act(async () => { retry.resolve(APPLIED); });
    await flush();
    const newest = surfaceCalls[surfaceCalls.length - 1];
    expect(newest.args.expanded).toBe(true);

    await act(async () => { newest.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('open');
  });

  it('bounds collapse retries and settles after the final native failure', async () => {
    await mount();
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    await act(async () => { surfaceCalls.find((c) => c.args.expanded)!.resolve(APPLIED); });
    await flush();
    surfaceCalls.length = 0;

    await act(async () => { current.onHoverEnd(); });
    await act(async () => { vi.advanceTimersByTime(COLLAPSE_DELAY_MS + SHRINK_DELAY_MS); });
    await flush();
    await act(async () => { surfaceCalls[0].reject(new Error('first failure')); });
    await flush();
    await act(async () => { vi.advanceTimersByTime(100); });
    await flush();
    await act(async () => { surfaceCalls[1].reject(new Error('second failure')); });
    await flush();
    await act(async () => { vi.advanceTimersByTime(300); });
    await flush();
    await act(async () => { surfaceCalls[2].reject(new Error('final failure')); });
    await flush();

    expect(surfaceCalls).toHaveLength(3);
    expect(current.phase).toBe('collapsed');
    expect(current.expanded).toBe(false);
  });

  it('shrinks immediately after the leave delay when reduced motion is preferred', async () => {
    vi.mocked(window.matchMedia).mockReturnValue({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList);
    await mount();

    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    await act(async () => { surfaceCalls.find((c) => c.args.expanded)!.resolve(APPLIED); });
    await flush();
    surfaceCalls.length = 0;

    await act(async () => { current.onHoverEnd(); });
    await act(async () => { vi.advanceTimersByTime(COLLAPSE_DELAY_MS + 1); });
    await flush();
    expect(surfaceCalls.some((c) => !c.args.expanded)).toBe(true);
  });
});

describe('overlay motion tokens', () => {
  it('derives the shrink delay from the height transition (heightMs + 20)', () => {
    expect(SHRINK_DELAY_MS).toBe(OVERLAY_HEIGHT_MS + 20);
  });
});
