import { useLayoutEffect, useRef } from 'react';
import { useDictationPartial } from '../../lib/hooks/useDictationPartial';

/** Visible transcript tail. The card is capped at three lines; older words
 *  scroll off the top rather than growing the window. */
const MAX_VISIBLE_CHARS = 320;

/**
 * Keep only the trailing words that can plausibly fit the card, breaking on a
 * word boundary so the first visible word is never sliced mid-token.
 *
 * The earlier overlay-wing version truncated the *head* with a CSS ellipsis,
 * which pinned the display to the first few characters of the recording
 * ("Oka…") forever. A live preview only means anything if it follows the
 * speaker, so the tail is what survives.
 */
export function visiblePartial(text: string): string {
  const trimmed = text.trim();
  if (trimmed.length <= MAX_VISIBLE_CHARS) return trimmed;
  const tail = trimmed.slice(trimmed.length - MAX_VISIBLE_CHARS);
  const boundary = tail.indexOf(' ');
  return boundary === -1 ? tail : tail.slice(boundary + 1);
}

/**
 * Presentational card. Renders nothing without text so the window never shows
 * an empty box — Rust also gates visibility, this is the second guard.
 */
export function DictationPreviewCard({ text }: { text: string }) {
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // Pin to the newest words as the transcript grows.
  useLayoutEffect(() => {
    const node = scrollRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [text]);

  if (!text) return null;

  return (
    <main className="flex w-full justify-center px-3" aria-label="Live dictation preview">
      <div className="flex w-full items-start gap-2.5 overflow-hidden rounded-[14px] border border-white/10 bg-[#141414]/95 px-3.5 py-2.5 text-white shadow-2xl backdrop-blur-3xl">
        <span
          aria-hidden="true"
          className="mt-[5px] h-2 w-2 shrink-0 animate-pulse rounded-full bg-red-500"
        />
        <div
          ref={scrollRef}
          role="status"
          aria-live="polite"
          aria-label="Words recognized so far"
          // The cap lives on the scrolling element itself: as a flex child it
          // would otherwise size to its content and spill past the card.
          // 64px ≈ three lines at 13px/leading-relaxed.
          className="max-h-[64px] min-h-0 min-w-0 flex-1 overflow-y-auto text-[13px] leading-relaxed text-white/85"
        >
          <p className="whitespace-pre-wrap break-words">{text}</p>
        </div>
      </div>
    </main>
  );
}

export function DictationPreviewApp() {
  return <DictationPreviewCard text={visiblePartial(useDictationPartial())} />;
}
