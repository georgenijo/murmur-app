import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useDictationPartial } from '../../lib/hooks/useDictationPartial';
import { useQueryAppearance } from '../../lib/hooks/useQueryAppearance';

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
 * bottom-aligns it so it hugs the floating pill beneath the window. */
export function DictationPreviewCard({ text, stableText = '', above = false }: {
  text: string; stableText?: string; above?: boolean;
}) {
  if (!text && !stableText) return null;
  return <main className="live-words-window" data-above={above || undefined} aria-label="Live dictation preview">
    <div className="live-words-card">
      <span className="live-words-dot" aria-hidden="true" />
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
  useEffect(() => {
    let disposed = false;
    let revision = 0;
    let unlisten: (() => void) | undefined;
    const refresh = async () => {
      const request = ++revision;
      try {
        const blob = await invoke<string | null>('load_settings_blob');
        const value: unknown = typeof blob === 'string' ? JSON.parse(blob) : null;
        const settings = value !== null && typeof value === 'object' && !Array.isArray(value)
          ? value as { overlayLiveWords?: unknown; overlayPlacement?: unknown }
          : null;
        // Live words default on: only an explicit `false` opts out.
        const consent = settings !== null && settings.overlayLiveWords !== false;
        if (!disposed && request === revision) {
          setEnabled(consent);
          setAbove(settings?.overlayPlacement === 'floating_pill' || settings?.overlayPlacement === 'below_menu_bar');
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
  return <DictationPreviewCard text={useDictationPartial(enabled)} above={above} />;
}
