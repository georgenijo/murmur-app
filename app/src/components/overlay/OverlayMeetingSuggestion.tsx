import type { OverlayGeometry } from '../../lib/overlayGeometry';
import type { MeetingSuggestion } from '../../lib/meetingSuggestions';

interface OverlayMeetingSuggestionProps {
  geometry: OverlayGeometry;
  expanded: boolean;
  suggestion: MeetingSuggestion;
  busy: boolean;
  error: string | null;
  onAccept: () => void;
  onDismiss: () => void;
}

export function OverlayMeetingSuggestion({
  geometry,
  expanded,
  suggestion,
  busy,
  error,
  onAccept,
  onDismiss,
}: OverlayMeetingSuggestionProps) {
  const stopPointerEvent = (event: React.SyntheticEvent) => event.stopPropagation();
  return (
    <section
      aria-hidden={!expanded}
      aria-label="Meeting suggestion"
      className="overlay-dropdown flex flex-col justify-center gap-3 px-4 pb-3 pt-2 text-white"
      onMouseDown={stopPointerEvent}
      onClick={stopPointerEvent}
      onDoubleClick={stopPointerEvent}
      style={{
        height: geometry.dropdownH,
        opacity: expanded ? 1 : 0,
        pointerEvents: expanded ? 'auto' : 'none',
        transition: 'opacity 200ms ease',
      }}
    >
      <p className="line-clamp-2 text-center text-xs font-medium leading-relaxed" title={`${suggestion.title} started. Start Notetaker?`}>
        {suggestion.title} started. Start Notetaker?
      </p>
      <div role="group" aria-label="Meeting suggestion actions" className="flex items-center justify-center gap-2">
        <button
          type="button"
          disabled={busy}
          onClick={onAccept}
          className="rounded-[9px] bg-cyan-300 px-4 py-1.5 text-xs font-semibold text-slate-950 transition-colors hover:bg-cyan-200 disabled:cursor-wait disabled:opacity-60"
        >
          {busy ? 'Working…' : 'Accept'}
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onDismiss}
          className="rounded-[9px] bg-white/10 px-4 py-1.5 text-xs font-medium text-white/85 transition-colors hover:bg-white/15 disabled:cursor-wait disabled:opacity-60"
        >
          Dismiss
        </button>
      </div>
      {error && <p role="alert" className="text-center text-[10px] text-red-300">{error}</p>}
    </section>
  );
}
