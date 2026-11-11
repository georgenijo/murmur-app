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
  // The floating pill's Bezel follows the system appearance, so its card takes
  // the Bezel ink; the notch island is always dark.
  const bezel = geometry.floating;
  return (
    <section
      aria-hidden={!expanded}
      aria-label="Meeting suggestion"
      className={`overlay-dropdown flex flex-col justify-center gap-3 px-4 pb-3 pt-2 ${bezel ? '' : 'bezel notch-bezel'}`}
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
          className="bezel-meeting-accept overlay-meeting-action font-semibold disabled:cursor-wait disabled:opacity-60"
        >
          {busy ? 'Working…' : 'Accept'}
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onDismiss}
          className="bezel-meeting-dismiss overlay-meeting-action font-medium disabled:cursor-wait disabled:opacity-60"
        >
          Dismiss
        </button>
      </div>
      {error && <p role="alert" className="bezel-meeting-error overlay-meeting-error text-center">{error}</p>}
    </section>
  );
}
