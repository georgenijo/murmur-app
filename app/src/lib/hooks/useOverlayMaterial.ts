import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { OverlayContent } from '../overlayGeometry';

export interface OverlayMaterialState {
  floating: boolean;
  /** The pill is faded in (not idle). */
  shown: boolean;
  /** The hover card is open, as CSS renders it. */
  expanded: boolean;
  content: OverlayContent;
}

/** Send `command` with the latest args, one request at a time: a burst
 * coalesces to its last value and can never land out of order. `native`
 * holds the last settled acknowledgment. */
function useLatestMaterialCommand<T extends Record<string, unknown>>(command: string) {
  const [native, setNative] = useState(false);
  const latest = useRef<T | null>(null);
  const queue = useRef({ running: false, dirty: false, mounted: true });
  useLayoutEffect(() => {
    const q = queue.current;
    q.mounted = true;
    return () => { q.mounted = false; };
  }, []);
  const send = useCallback((args: T) => {
    latest.current = args;
    const q = queue.current;
    q.dirty = true;
    if (q.running) return;
    q.running = true;
    void (async () => {
      while (q.dirty && q.mounted) {
        q.dirty = false;
        let applied = false;
        try {
          applied = await invoke<boolean>(command, latest.current ?? {}) === true;
        } catch { applied = false; }
        if (q.mounted && !q.dirty) setNative(applied);
      }
      q.running = false;
    })();
  }, [command]);
  return [native, send] as const;
}

/** Tell Rust which shape the floating pill shows, so the native Bezel material
 * behind the webview follows it on the island's own curve. Rust derives the
 * rect from its geometry contract; this sends only semantic state. Returns
 * true while a native material backs the pill. */
export function useOverlayMaterial({ floating, shown, expanded, content }: OverlayMaterialState): boolean {
  const [native, send] = useLatestMaterialCommand<{ floating: boolean; shown: boolean; expanded: boolean; content: OverlayContent }>('set_overlay_material');
  useLayoutEffect(() => {
    send({ floating, shown, expanded, content });
  }, [send, floating, shown, expanded, content]);
  return native && floating;
}

/** The Live Words card's laid-out size, for Rust to clip its material to.
 * Rust places it (bottom-centred, lifted with the pill), so only the size,
 * which follows the words, comes from the webview. */
export function useLiveWordsMaterial(enabled: boolean) {
  const [native, send] = useLatestMaterialCommand<{ width: number; height: number }>('set_live_words_material');
  const report = useCallback((width: number, height: number) => {
    send({ width, height });
  }, [send]);
  return { native: native && enabled, report: enabled ? report : undefined };
}
