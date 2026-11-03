import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { emitTo, listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useDictationPartial } from '../../lib/hooks/useDictationPartial';
import { useQueryAppearance } from '../../lib/hooks/useQueryAppearance';
import { parseSettingsBlob } from '../../lib/settings';
import { LIVE_WORDS_LIFT_EVENT, LIVE_WORDS_LIFT_REQUEST_EVENT, liveWordsLiftTransition } from '../../lib/overlayMotion';
import { useLiveWordsMaterial } from '../../lib/hooks/useOverlayMaterial';

const MAX_VISIBLE_CHARS = 320;
export function visiblePartial(text: string): string {
  const trimmed = text.trim();
  if (trimmed.length <= MAX_VISIBLE_CHARS) return trimmed;
  const tail = trimmed.slice(-MAX_VISIBLE_CHARS);
  const boundary = tail.indexOf(' ');
  return boundary === -1 ? tail : tail.slice(boundary + 1);
}

/** Presentation only: no copy action, title, export, persistence or telemetry.
 * Stable text is only accepted explicitly from an authoritative stream. Today's
 * whole-string partial API has no commit boundary, so all of it is tentative.
 * The card wraps up to three lines and keeps the newest words in view; `above`
 * bottom-aligns it so it hugs the floating pill beneath the window. It carries
 * no recording dot: the island already shows recording state. `lift` moves the
 * card clear of the island's hover dropdown, on the island's own curve.
 * Above the pill the card wears the pill's Bezel material: `onCardSize` hands
 * its laid-out size to Rust, which clips the native blur to it, and
 * `material` drops the webview's own fill once that blur is in place. */
export function DictationPreviewCard({ text, stableText = '', above = false, lift = 0, material = false, onCardSize }: {
  text: string; stableText?: string; above?: boolean; lift?: number;
  material?: boolean; onCardSize?: (width: number, height: number) => void;
}) {
  const card = useRef<HTMLDivElement>(null);
  const visible = Boolean(text || stableText);
  useLayoutEffect(() => {
    const el = card.current;
    if (!onCardSize) return;
    if (!el) { onCardSize(0, 0); return; }
    // Layout size: the lift transform belongs to Rust's placement.
    const report = () => onCardSize(el.offsetWidth, el.offsetHeight);
    report();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(report);
    observer.observe(el);
    return () => { observer.disconnect(); onCardSize(0, 0); };
  }, [onCardSize, visible]);
  if (!visible) return null;
  return <main className="live-words-window" data-above={above || undefined} aria-label="Live dictation preview">
    <div ref={card} className={`live-words-card${above ? ' bezel bezel-surface' : ''}`}
      data-material={above && material ? 'native' : undefined} style={{
      transform: lift ? `translateY(${above ? -lift : lift}px)` : undefined,
      transition: liveWordsLiftTransition(above),
    }}>
      <div className="live-words-text" role="status" aria-live="polite" aria-label="Words recognized so far">
        <p>
          {stableText && <><span className="live-words-stable">{visiblePartial(stableText)}</span>{' '}</>}
          {text && <span className="live-words-tentative">{visiblePartial(text)}</span>}
        </p>
      </div>
    </div>
  </main>;
}

export function DictationPreviewApp() {
  useQueryAppearance();
  const [enabled, setEnabled] = useState(false);
  const [above, setAbove] = useState(false);
  const [lift, setLift] = useState(0);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<unknown>(LIVE_WORDS_LIFT_EVENT, ({ payload }) => {
      if (!disposed) setLift(typeof payload === 'number' && Number.isFinite(payload) && payload > 0 ? payload : 0);
    }).then(fn => {
      if (disposed) { fn(); return; }
      unlisten = fn;
      // Subscribed first, so the reply cannot be missed.
      emitTo('overlay', LIVE_WORDS_LIFT_REQUEST_EVENT).catch(() => {});
    }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, []);
  useEffect(() => {
    let disposed = false;
    let revision = 0;
    let unlisten: (() => void) | undefined;
    const refresh = async () => {
      const request = ++revision;
      try {
        const blob = await invoke<string | null>('load_settings_blob');
        // `null` covers both a fresh install and a quarantined corrupt file, so
        // it fails closed. The main window seeds a fresh install's document,
        // whose commit event re-runs this read.
        if (typeof blob !== 'string') throw new Error('no durable settings');
        JSON.parse(blob); // parseSettingsBlob would fall back to defaults.
        // Same migrations and defaults as Settings, without persisting them.
        const settings = parseSettingsBlob(blob);
        if (!disposed && request === revision) {
          setEnabled(settings.overlayLiveWords);
          setAbove(settings.overlayPlacement === 'floating_pill');
        }
      } catch { if (!disposed && request === revision) setEnabled(false); }
    };
    void listen<unknown>('live-words-consent-persisted', ({ payload }) => {
      if (disposed) return;
      // Opt-out clears immediately. A true event alone never enables words:
      // re-read the durable source, ignoring delayed WebKit cache propagation.
      if (payload !== true) setEnabled(false);
      void refresh();
    }).then(fn => {
      if (disposed) fn();
      else { unlisten = fn; void refresh(); } // Subscribe before reading.
    }).catch(() => { if (!disposed) setEnabled(false); });
    return () => { disposed = true; revision++; unlisten?.(); };
  }, []);
  const material = useLiveWordsMaterial(above);
  return <DictationPreviewCard text={useDictationPartial(enabled)} above={above} lift={lift}
    material={material.native} onCardSize={material.report} />;
}
