import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act, StrictMode } from 'react';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { NativeWindowLifecycle, startWindowLifecycle } from './windowLifecycle';

let callbacks = new Map<number, FrameRequestCallback>();
let id = 0;
let cleanup: (() => void) | undefined;
let root: Root | undefined;
const step = async () => {
  const current = [...callbacks.values()];
  callbacks.clear();
  current.forEach((callback) => callback(performance.now()));
  await Promise.resolve();
  await Promise.resolve();
};
const settle = async () => { for (let count = 0; count < 8; count++) await Promise.resolve(); };

beforeEach(() => {
  callbacks = new Map();
  vi.stubGlobal('matchMedia', () => ({ addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  vi.stubGlobal('isTauri', true);
  vi.stubGlobal('__TAURI_INTERNALS__', { metadata: {} });
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callbacks.set(++id, callback); return id; });
  vi.stubGlobal('cancelAnimationFrame', (key: number) => callbacks.delete(key));
  // jsdom does not lay out CSS; browser tests cover the real token measurement.
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ height: 52 } as DOMRect);
  document.documentElement.style.setProperty('--murmur-background', '#f7fafc');
});
afterEach(async () => {
  cleanup?.();
  cleanup = undefined;
  if (root) await act(async () => root?.unmount());
  root = undefined;
  clearMocks();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.documentElement.removeAttribute('style');
});

describe('workspace window lifecycle', () => {
  it('waits for a committed React tree and two frames before revealing', async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    mockIPC((command, args) => { if (command === 'sync_window_chrome') send(args); });
    const host = document.createElement('div');
    root = createRoot(host);
    await act(async () => root!.render(<StrictMode><NativeWindowLifecycle><div>Paint me</div></NativeWindowLifecycle></StrictMode>));
    await settle();
    expect(host.textContent).toBe('Paint me');
    expect(send.mock.calls.map(([args]) => args.ready)).toEqual([false]);
    await step();
    expect(send.mock.calls.map(([args]) => args.ready)).toEqual([false]);
    await step();
    await settle();
    expect(send.mock.calls.map(([args]) => args.ready)).toEqual([false, true]);
  });

  it('synchronizes exact light, dark and custom theme colors without re-showing diagnostics', async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    cleanup = startWindowLifecycle(send, false);
    await settle();
    expect(send).toHaveBeenLastCalledWith({ background: '#f7fafc', headerHeight: 52, zoom: 1 }, false);
    document.documentElement.style.setProperty('--murmur-background', '#12181c');
    document.documentElement.dataset.appearance = 'dark';
    await settle();
    await step();
    await step();
    await settle();
    expect(send).toHaveBeenLastCalledWith({ background: '#12181c', headerHeight: 52, zoom: 1 }, false);
    document.documentElement.style.setProperty('--murmur-background', '#123456');
    await settle(); await step(); await settle();
    expect(send).toHaveBeenLastCalledWith({ background: '#123456', headerHeight: 52, zoom: 1 }, false);
    expect(send.mock.calls.every(([, ready]) => ready === false)).toBe(true);
  });

  it('theme mutations during initial paint do not cancel readiness', async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    cleanup = startWindowLifecycle(send, true);
    await step();
    document.documentElement.dataset.appearance = 'light';
    await settle(); await step(); await settle();
    expect(send.mock.calls.some(([, ready]) => ready)).toBe(true);
  });

  it('serializes native updates so a slow old theme cannot overwrite a new one', async () => {
    let resolveFirst!: () => void;
    const send = vi.fn().mockImplementationOnce(() => new Promise<void>((resolve) => { resolveFirst = resolve; })).mockResolvedValue(undefined);
    cleanup = startWindowLifecycle(send, true);
    await settle();
    document.documentElement.style.setProperty('--murmur-background', '#12181c');
    await settle(); await step(); await step(); await settle();
    expect(send).toHaveBeenCalledTimes(1);
    resolveFirst(); await settle();
    expect(send.mock.calls[send.mock.calls.length - 1]).toEqual([{ background: '#12181c', headerHeight: 52, zoom: 1 }, true]);
  });

  it('updates page zoom and supports reset without scaling the native x inset', async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    cleanup = startWindowLifecycle(send, true);
    await step(); await step(); await settle();
    const key = new KeyboardEvent('keydown', { key: '+', metaKey: true, cancelable: true });
    window.dispatchEvent(key);
    await settle();
    expect(key.defaultPrevented).toBe(true);
    expect(send.mock.calls[send.mock.calls.length - 1]?.[0].zoom).toBe(1.1);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: '0', metaKey: true }));
    await settle();
    expect(send.mock.calls[send.mock.calls.length - 1]?.[0].zoom).toBe(1);
  });

  it('publishes bounded page zoom for the CSS inset and resets it without an observer loop', async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    cleanup = startWindowLifecycle(send, false);
    for (let count = 0; count < 10; count++) {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: '-', metaKey: true }));
    }
    await settle(); await step(); await step(); await settle();
    expect(document.documentElement.style.getPropertyValue('--ui-window-page-zoom')).toBe('0.5');
    expect(send.mock.lastCall).toEqual([{ background: '#f7fafc', headerHeight: 52, zoom: 0.5 }, false]);
    const calls = send.mock.calls.length;
    await step(); await step(); await settle();
    expect(send).toHaveBeenCalledTimes(calls);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: '0', ctrlKey: true }));
    await settle();
    expect(document.documentElement.style.getPropertyValue('--ui-window-page-zoom')).toBe('1');
    for (let count = 0; count < 15; count++) {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: '+', ctrlKey: true }));
      await settle();
    }
    await settle();
    expect(document.documentElement.style.getPropertyValue('--ui-window-page-zoom')).toBe('2');
    expect(send.mock.lastCall?.[0].zoom).toBe(2);
  });

  it('restores the previous CSS zoom property on disposal, including StrictMode remounts', () => {
    const style = document.documentElement.style;
    style.setProperty('--ui-window-page-zoom', '1.2', 'important');
    const stop = startWindowLifecycle(vi.fn().mockResolvedValue(undefined), false);
    expect(style.getPropertyValue('--ui-window-page-zoom')).toBe('1');
    stop();
    expect(style.getPropertyValue('--ui-window-page-zoom')).toBe('1.2');
    expect(style.getPropertyPriority('--ui-window-page-zoom')).toBe('important');
  });

  it('cancels pending reveal and observations when a mount is disposed', async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    const stop = startWindowLifecycle(send, true);
    await settle();
    stop();
    document.documentElement.style.setProperty('--murmur-background', '#123456');
    await settle(); await step(); await step(); await settle();
    expect(send).toHaveBeenCalledTimes(1);
    expect(send.mock.calls[0][1]).toBe(false);
  });
});
