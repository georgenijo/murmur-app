import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { DictationStatus } from '../types';

/** Bars in the notch's right wing. */
export const NOTCH_BAR_COUNT = 7;
/** Bars in the floating pill. */
export const PILL_BAR_COUNT = 18;
/** Resting bar scale: a short tick rather than an empty slot. */
const REST_SCALE = 0.14;

export interface OverlayWaveform {
  /** Attach to each bar element via `ref={el => { barRefs.current[i] = el; }}`. */
  barRefs: React.MutableRefObject<(HTMLDivElement | null)[]>;
}

/**
 * Owns the `audio-level` listener and the per-frame bar animation. Bars keep a
 * fixed layout box and animate only `transform: scaleY`, written directly to
 * the DOM from rAF: no React render, layout, or paint invalidation per frame.
 * The loop runs only while recording.
 */
export function useWaveform(status: DictationStatus): OverlayWaveform {
  const audioLevelRef = useRef(0);
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<number>('audio-level', (event) => {
      audioLevelRef.current = event.payload;
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    }).catch(() => {
      // Non-Tauri previews do not expose the native event bridge.
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  useEffect(() => {
    const bars = () => barRefs.current.filter((el): el is HTMLDivElement => el !== null);
    if (status !== 'recording') {
      audioLevelRef.current = 0;
      for (const el of bars()) {
        el.style.transition = 'transform 300ms ease-out';
        el.style.transform = `scaleY(${REST_SCALE})`;
      }
      return;
    }
    for (const el of bars()) el.style.transition = 'transform 60ms linear';
    let rafId: number;
    const animate = () => {
      const level = Math.min(1, audioLevelRef.current * 16);
      const current = bars();
      const center = (current.length - 1) / 2 || 1;
      current.forEach((el, i) => {
        const baseline = 0.08 + Math.random() * 0.07;
        const envelope = 0.5 + 0.5 * (1 - Math.abs(i - center) / center);
        const boost = level * level * 0.4 * Math.random();
        const h = Math.max(REST_SCALE, Math.min(1, baseline + level * envelope + boost));
        el.style.transform = `scaleY(${h.toFixed(3)})`;
      });
      rafId = requestAnimationFrame(animate);
    };
    rafId = requestAnimationFrame(animate);
    return () => cancelAnimationFrame(rafId);
  }, [status]);

  return { barRefs };
}
