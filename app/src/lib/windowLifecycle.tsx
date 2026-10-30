import { useEffect, type PropsWithChildren } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';

export interface WindowAppearance {
  background: string;
  headerHeight: number;
  zoom: number;
}

// Resolve lengths through layout: the header token can be rem, px or calc().
// Rust never duplicates the CSS header height.
export function measureWindowAppearance(zoom = 1): WindowAppearance {
  const probe = document.createElement('div');
  probe.style.cssText = 'position:fixed;visibility:hidden;pointer-events:none;height:var(--ui-window-header-height);width:0';
  document.body.append(probe);
  const headerHeight = probe.getBoundingClientRect().height;
  probe.remove();
  return {
    background: getComputedStyle(document.documentElement).getPropertyValue('--murmur-background').trim(),
    headerHeight,
    zoom,
  };
}

type Send = (appearance: WindowAppearance, ready: boolean) => Promise<unknown>;

/** Two animation frames place readiness after a browser paint. Theme writes
 * are observed at their source, including System and imported-theme changes.
 * Calls are serialized so an older color cannot win an IPC race. */
export function startWindowLifecycle(send: Send, showAfterPaint: boolean): () => void {
  let disposed = false;
  let ready = false;
  let zoom = 1;
  let frame = 0;
  let paintFrame = 0;
  let previous = '';
  let pending = Promise.resolve<unknown>(undefined);
  const sync = () => {
    if (disposed) return;
    const appearance = measureWindowAppearance(zoom);
    const key = JSON.stringify([appearance, ready]);
    if (key === previous) return;
    previous = key;
    pending = pending.catch(() => {}).then(() => {
      if (disposed) return;
      return send(appearance, ready).catch(() => { previous = ''; });
    });
  };
  const schedule = () => {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(sync);
  };
  const observer = new MutationObserver(schedule);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['style', 'class', 'data-appearance'] });
  const media = window.matchMedia('(prefers-color-scheme: dark)');
  media.addEventListener('change', schedule);
  window.addEventListener('resize', schedule);
  window.visualViewport?.addEventListener('resize', schedule);
  const onKey = (event: KeyboardEvent) => {
    if (!(event.metaKey || event.ctrlKey) || event.altKey || !['=', '+', '-', '0'].includes(event.key)) return;
    event.preventDefault();
    zoom = event.key === '0' ? 1 : Math.min(2, Math.max(0.5, Math.round((zoom + (event.key === '-' ? -0.1 : 0.1)) * 10) / 10));
    sync();
  };
  window.addEventListener('keydown', onKey);
  sync();
  // The caller starts this from a committed React effect, or waits for the
  // diagnostics root to mount. One rAF alone runs before the first paint.
  paintFrame = requestAnimationFrame(() => {
    paintFrame = requestAnimationFrame(() => {
      ready = showAfterPaint;
      sync();
    });
  });
  return () => {
    disposed = true;
    cancelAnimationFrame(frame);
    cancelAnimationFrame(paintFrame);
    observer.disconnect();
    media.removeEventListener('change', schedule);
    window.removeEventListener('resize', schedule);
    window.visualViewport?.removeEventListener('resize', schedule);
    window.removeEventListener('keydown', onKey);
  };
}

export function startNativeWindowLifecycle(showAfterPaint: boolean): () => void {
  if (!isTauri()) return () => {};
  return startWindowLifecycle((appearance, ready) => invoke('sync_window_chrome', { ...appearance, ready }), showAfterPaint);
}

export function NativeWindowLifecycle({ children }: PropsWithChildren) {
  useEffect(() => startNativeWindowLifecycle(true), []);
  return children;
}
