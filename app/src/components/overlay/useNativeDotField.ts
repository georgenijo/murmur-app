import { useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { DotScopeGeometry, ProcessingStyle, ScopeStage } from './dotScope';

/** Native owns decorative frames on macOS; fixtures/unsupported renderers use
 * the same bounded Canvas simulation. No text crosses this presentation seam. */
export function useNativeDotField(stage: ScopeStage, style: ProcessingStyle,
  geometry: DotScopeGeometry | undefined, fixtureAt: number | undefined) {
  const [native, setNative] = useState(() => fixtureAt === undefined && isTauri());
  const latest = useRef({ stage, style }); latest.current = { stage, style };
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const updateRef = useRef<(() => void) | null>(null);
  useEffect(() => {
    if (!native) return;
    let disposed = false, visible = true;
    const media = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    const update = () => {
      const state = { ...latest.current, reduced: media?.matches ?? false, visible: visible && !document.hidden };
      queue.current = queue.current.catch(() => {}).then(async () => {
        if (disposed) return;
        try {
          if (!await invoke<boolean>('set_overlay_field_state', state) && !disposed) setNative(false);
        } catch { if (!disposed) setNative(false); }
      });
    };
    updateRef.current = update;
    let stop: (() => void) | undefined;
    void listen<boolean>('overlay-visible-changed', ({ payload }) => {
      visible = payload; update();
    }).then(fn => { if (disposed) fn(); else stop = fn; }).catch(() => {});
    document.addEventListener('visibilitychange', update);
    window.addEventListener('resize', update);
    media?.addEventListener('change', update);
    update();
    return () => {
      disposed = true; stop?.(); updateRef.current = null;
      document.removeEventListener('visibilitychange', update);
      window.removeEventListener('resize', update); media?.removeEventListener('change', update);
      // Serialize teardown behind pending state, so a late acknowledgement
      // cannot resurrect the view after Canvas fallback or unmount.
      queue.current = queue.current.catch(() => {}).then(() => invoke('set_overlay_field_state', {
        stage: 'idle', style: 'flow', reduced: true, visible: false,
      })).catch(() => {});
    };
  }, [native]);
  useEffect(() => { updateRef.current?.(); }, [stage, style, geometry]);
  return native;
}
