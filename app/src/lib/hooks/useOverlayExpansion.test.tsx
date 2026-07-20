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

// A pending set_overlay_surface invocation the test controls.
interface SurfaceCall {
  args: { expanded: boolean; previewVisible: boolean };
  resolve: (v: { windowW: number; windowH: number }) => void;
  reject: (e: unknown) => void;
}

const APPLIED = { windowW: 305, windowH: 76 };

describe('useOverlayExpansion', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: OverlayExpansion;
  let surfaceCalls: SurfaceCall[];
  const listeners = new Map<string, (e: { payload: unknown }) => void>();

  function Harness(props: { previewRowVisible?: boolean; disabled?: boolean; withIsland?: boolean }) {
    current = useOverlayExpansion({
      previewRowVisible: props.previewRowVisible ?? false,
      disabled: props.disabled ?? false,
    });
    return props.withIsland ? <div ref={current.islandRef} /> : null;
  }

  function emitEvent(event: string, payload: unknown) {
    listeners.get(event)?.({ payload });
  }

  function surfaceCallCount() {
    return mocks.invoke.mock.calls.filter((c) => c[0] === 'set_overlay_surface').length;
  }

  async function flush() {
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
  }

  async function mount(props: { previewRowVisible?: boolean; disabled?: boolean; withIsland?: boolean } = {}) {
    // Fresh root each mount so a test can unmount and re-mount cleanly.
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => { root.render(<Harness {...props} />); });
    // Resolve the mount-time preview-sync resize so the serial writer is idle, then
    // reset the tracking array so each test's first surface call starts at index 0.
    await act(async () => {
      surfaceCalls.forEach((c) => c.resolve(APPLIED));
      await Promise.resolve();
    });
    surfaceCalls.length = 0;
  }

  beforeEach(() => {
    vi.useFakeTimers();
    surfaceCalls = [];
    listeners.clear();

    mocks.invoke.mockReset();
    mocks.invoke.mockImplementation((cmd: string, args: unknown) => {
      if (cmd === 'set_overlay_surface') {
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

    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => { root.unmount(); });
    container.remove();
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

  it('display change mid-open cancels timers and forces collapsed', async () => {
    await mount();
    // Open fully.
    await act(async () => { current.onHoverStart(); });
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS); });
    await flush();
    await act(async () => { surfaceCalls.find((c) => c.args.expanded)!.resolve(APPLIED); });
    await flush();
    expect(current.phase).toBe('open');

    // Arm a close, then a display change arrives.
    await act(async () => { current.onHoverEnd(); });
    await act(async () => { emitEvent('overlay-geometry-changed', {}); });
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

    await act(async () => { root.unmount(); });

    // Nothing new should invoke after unmount, even as timers advance and the
    // in-flight ack resolves.
    await act(async () => { vi.advanceTimersByTime(SHRINK_DELAY_MS * 4); });
    await act(async () => { pending!.resolve(APPLIED); await Promise.resolve(); });
    expect(surfaceCallCount()).toBe(before);
  });

  it('poller performs no IPC while the overlay is hidden or the app is disabled', async () => {
    // Disabled at mount → gated.
    await mount({ withIsland: true, disabled: true });
    mocks.cursorPosition.mockClear();
    mocks.outerPosition.mockClear();
    const surfacesBefore = surfaceCallCount();
    await act(async () => { vi.advanceTimersByTime(HOVER_OPEN_DWELL_MS * 4); });
    await flush();
    expect(mocks.cursorPosition).not.toHaveBeenCalled();
    expect(mocks.outerPosition).not.toHaveBeenCalled();
    expect(surfaceCallCount()).toBe(surfacesBefore);

    // Re-mount enabled but hidden → still gated.
    await act(async () => { root.unmount(); });
    surfaceCalls = [];
    await mount({ withIsland: true, disabled: false });
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
});

describe('overlay motion tokens', () => {
  it('derives the shrink delay from the height transition (heightMs + 20)', () => {
    expect(SHRINK_DELAY_MS).toBe(OVERLAY_HEIGHT_MS + 20);
  });
});
