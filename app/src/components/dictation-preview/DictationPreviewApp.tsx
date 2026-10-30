import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useDictationPartial } from '../../lib/hooks/useDictationPartial';
import { loadSettings } from '../../lib/settings';
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
 * whole-string partial API has no commit boundary, so all of it is tentative. */
export function DictationPreviewCard({ text, stableText = '' }: { text: string; stableText?: string }) {
  if (!text && !stableText) return null;
  return <main className="live-words-window" aria-label="Live dictation preview">
    <div className="live-words-pill" role="status" aria-label="Words recognized so far">
      {stableText && <span className="live-words-stable"><span>{visiblePartial(stableText)}</span></span>}
      {text && <span className="live-words-tentative">{text}</span>}
    </div>
  </main>;
}

export function DictationPreviewApp() {
  useQueryAppearance();
  const [enabled, setEnabled] = useState(() => loadSettings().overlayLiveWords);
  useEffect(() => {
    let disposed = false;
    const stop = listen('settings-changed', () => {
      if (!disposed) setEnabled(loadSettings().overlayLiveWords);
    }).catch(() => () => {});
    return () => { disposed = true; void stop.then((fn) => fn()).catch(() => {}); };
  }, []);
  return <DictationPreviewCard text={useDictationPartial(enabled)} />;
}
