import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { DictationStatus } from '../types';

/** Number of animated waveform bars in the top-bar right slot. */
export const BAR_COUNT = 7;

export interface OverlayWaveform {
  /** Attach to each bar element via `ref={el => { barRefs.current[i] = el; }}`. */
  barRefs: React.MutableRefObject<(HTMLDivElement | null)[]>;
}

/**
 * Owns the audio-level listener and the rAF bar-height animation. Bars are
 * updated via direct DOM writes (no React state per frame) — unchanged from
 * the original inline implementation, just relocated.
 */
export function useWaveform(status: DictationStatus): OverlayWaveform {
  const audioLevelRef = useRef(0);
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);

  // Subscribe to audio level events from Rust (store in ref, no state update)
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<number>('audio-level', (event) => {
      audioLevelRef.current = event.payload;
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Animate waveform bars via rAF (direct DOM updates, no React reconciliation)
  useEffect(() => {
    if (status !== 'recording') {
      barRefs.current.forEach(el => {
        if (el) el.style.height = '2px';
      });
      return;
    }
    let rafId: number;
    const animate = () => {
      const level = Math.min(1, audioLevelRef.current * 16);
      barRefs.current.forEach((el, i) => {
        if (!el) return;
        const baseline = 0.08 + Math.random() * 0.07;
        const center = (BAR_COUNT - 1) / 2;
        const distFromCenter = 1 - Math.abs(i - center) / center;
        const envelope = 0.5 + 0.5 * distFromCenter;
        const reactiveHeight = level * envelope;
        const boost = level * level * 0.4 * Math.random();
        const h = Math.min(1, baseline + reactiveHeight + boost);
        el.style.height = `${Math.max(2, Math.round(h * 14))}px`;
      });
      rafId = requestAnimationFrame(animate);
    };
    rafId = requestAnimationFrame(animate);
    return () => cancelAnimationFrame(rafId);
  }, [status]);

  return { barRefs };
}
