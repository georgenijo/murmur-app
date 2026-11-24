import type { OverlayGeometry } from '../../lib/overlayGeometry';
import { PILL_FIT_PAD, PILL_FIT_TEXT_X } from '../../lib/overlayPillFit';

/** Lines the unrolled wheel shows above the front line. Rust sizes the card
 * for them (`TRANSCRIPT_DROPDOWN_H` in overlay.rs). */
export const TRANSCRIPT_LINES = 3;
/** Opacity of each unrolled line, nearest the front line first: older lines
 * recede, like the wheel they rolled away on. */
export const TRANSCRIPT_LINE_OPACITY = [0.6, 0.38, 0.22] as const;
/** The front line's 16px box is centred in the 32px row, 8px below its top;
 * the card reaches that far into the row so the nearest line sits right on
 * top of it, at the wheel's pitch. */
const TRANSCRIPT_TUCK = 8;

/**
 * The floating pill's hover card while dictating with Live Words: the wheel
 * unrolled. The lines that rolled away come back flat above the front line,
 * left-aligned with it, dimmer the older they are. The front line stays where
 * it is, in the pill's own row. Screen readers already hear the words from
 * the wheel's live region, so this is decoration.
 */
export function OverlayTranscript({ geometry, expanded, lines }: {
  geometry: OverlayGeometry;
  expanded: boolean;
  /** Finished lines, oldest first. */
  lines: string[];
}) {
  const first = Math.max(0, lines.length - TRANSCRIPT_LINES);
  const shown = lines.slice(first);
  return (
    <div className="bezel-transcript" data-testid="overlay-transcript" aria-hidden="true" style={{
      height: geometry.dropdownH + TRANSCRIPT_TUCK,
      marginBottom: -TRANSCRIPT_TUCK,
      paddingLeft: PILL_FIT_TEXT_X,
      paddingRight: PILL_FIT_PAD,
      opacity: expanded ? 1 : 0,
      transition: 'opacity 200ms ease',
      transitionDelay: expanded ? '100ms' : '0ms',
    }}>
      {shown.map((line, i) => (
        <div key={first + i} className="bezel-transcript-line"
          style={{ opacity: TRANSCRIPT_LINE_OPACITY[shown.length - 1 - i] }}>{line}</div>
      ))}
    </div>
  );
}
