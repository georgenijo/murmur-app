import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DotField, paintDots } from './DotField';
import { DotScope } from './dotScope';

const events = vi.hoisted(() => ({ listeners: new Map<string, (e: { payload: unknown }) => void>(), stops: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (name, fn) => {
  events.listeners.set(name, fn); return () => { events.listeners.delete(name); events.stops(); };
}) }));

const context = () => Object.fromEntries(['clearRect', 'save', 'translate', 'scale', 'beginPath', 'roundRect', 'clip', 'moveTo', 'arc', 'fill', 'restore', 'setTransform'].map(name => [name, vi.fn()]));

describe('canvas field lifecycle', () => {
  let root: Root; let container: HTMLDivElement; let reduced = false;
  let now = 0; let next = 0;
  let motion: (() => void) | undefined;
  const frames = new Map<number, FrameRequestCallback>();
  const tick = (count = 1) => {
    for (let i = 0; i < count; i++) {
      now += 1000 / 60;
      const callbacks = [...frames.values()]; frames.clear();
      callbacks.forEach(fn => fn(now));
    }
  };
  beforeEach(() => {
    reduced = false; now = 0; next = 0; motion = undefined; frames.clear(); events.listeners.clear(); events.stops.mockClear();
    Object.defineProperty(document, 'hidden', { configurable: true, value: false });
    vi.stubGlobal('requestAnimationFrame', (fn: FrameRequestCallback) => { frames.set(++next, fn); return next; });
    vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id));
    vi.stubGlobal('matchMedia', () => ({ get matches() { return reduced; }, addEventListener: (_: string, fn: () => void) => { motion = fn; }, removeEventListener: vi.fn() }));
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context() as unknown as CanvasRenderingContext2D);
    container = document.createElement('div'); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount()); container.remove();
    Object.defineProperty(document, 'hidden', { configurable: true, value: false });
    vi.restoreAllMocks(); vi.unstubAllGlobals();
  });

  it('does not keep an idle frame loop and paints at Retina backing resolution', async () => {
    vi.stubGlobal('devicePixelRatio', 2);
    await act(async () => root.render(<DotField stage="idle" />));
    tick(2); expect(frames.size).toBe(0);
    const canvas = container.querySelector('canvas')!;
    expect([canvas.width, canvas.height]).toEqual([224, 52]);
    expect(canvas.getAttribute('aria-hidden')).toBe('true');
    await act(async () => root.render(<DotField stage="recording" />));
    tick(2); expect(frames.size).toBe(1);
  });

  it('gates hidden work, restarts without catch-up, and releases every listener on unmount', async () => {
    const step = vi.spyOn(DotScope.prototype, 'advance');
    await act(async () => root.render(<DotField stage="recording" />));
    tick(3);
    events.listeners.get('overlay-visible-changed')?.({ payload: false });
    const count = step.mock.calls.length; tick(120); expect(step).toHaveBeenCalledTimes(count); expect(frames.size).toBe(0);
    events.listeners.get('overlay-visible-changed')?.({ payload: true }); tick();
    expect(step.mock.calls[step.mock.calls.length - 1]?.[0]).toBe(1 / 60);
    Object.defineProperty(document, 'hidden', { configurable: true, value: true }); document.dispatchEvent(new Event('visibilitychange'));
    tick(); expect(frames.size).toBe(0);
    await act(async () => root.unmount()); root = createRoot(container);
    expect(events.listeners.size).toBe(0); expect(events.stops).toHaveBeenCalledTimes(2); expect(frames.size).toBe(0);
  });

  it('keeps a native-hidden field asleep across stage and style changes', async () => {
    await act(async () => root.render(<DotField stage="recording" />));
    tick(2);
    events.listeners.get('overlay-visible-changed')?.({ payload: false });
    for (const style of ['sweep', 'flow', 'pulse'] as const) {
      await act(async () => root.render(<DotField stage="processing" processingStyle={style} />));
      tick(3); expect(frames.size).toBe(0);
    }
    events.listeners.get('overlay-visible-changed')?.({ payload: true });
    tick(2); expect(frames.size).toBe(1);
  });

  it('resizes backing pixels on display scale changes without replacing the simulation', async () => {
    vi.stubGlobal('devicePixelRatio', 1);
    const step = vi.spyOn(DotScope.prototype, 'advance');
    await act(async () => root.render(<DotField stage="recording" />));
    tick(2);
    const scope = step.mock.contexts[0];
    const canvas = container.querySelector('canvas')!;
    expect(canvas.width).toBe(112);
    vi.stubGlobal('devicePixelRatio', 2);
    await act(async () => window.dispatchEvent(new Event('resize')));
    tick(2);
    expect([canvas.width, canvas.height]).toEqual([224, 52]);
    expect(step.mock.contexts.every(context => context === scope)).toBe(true);
    vi.stubGlobal('devicePixelRatio', 1);
    await act(async () => root.render(<DotField stage="recording" geometry={{ ...(scope as DotScope).geometry }} />));
    tick(); expect(canvas.width).toBe(112);
    expect(step.mock.contexts.every(context => context === scope)).toBe(true);
  });

  it('Reduce Motion fades once then sleeps, and a preference change wakes the field', async () => {
    reduced = true;
    await act(async () => root.render(<DotField stage="recording" />)); tick(120);
    expect(frames.size).toBe(0); expect(container.querySelector('canvas')?.style.filter).toBe('none');
    reduced = false; motion?.(); tick(2); expect(frames.size).toBe(1);
    await act(async () => root.render(<DotField stage="idle" />)); tick(120); expect(frames.size).toBe(0);
  });

  it('wipes the dim dot-grid floor as well as the traces from the centre outward', () => {
    const scope = new DotScope();
    for (let i = 0; i < 120; i++) scope.advance(1 / 60, 'processing', -120, false);
    scope.advance(0.05, 'done', -120, false);
    scope.advance(0.05, 'done', -120, false);
    paintDots(context() as unknown as CanvasRenderingContext2D, scope, 1 / 60, 'flow', false);
    expect(scope.buckets[4 * 43 + 21]).toBe(0);
    expect(scope.buckets[4 * 43 + 32]).toBeGreaterThan(0);
  });

  it('does not subscribe or schedule a live loop in deterministic fixtures', async () => {
    await act(async () => root.render(<DotField stage="recording" fixtureAt={1.2} />));
    expect(frames.size).toBe(0); expect(events.listeners.size).toBe(0);
  });
});
