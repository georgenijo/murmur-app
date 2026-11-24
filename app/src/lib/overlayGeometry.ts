export interface IslandMetrics {
  notchW: number; notchH: number;
  cornerIdle: number; cornerActive: number;
  pillW: number; pillH: number;
}
export interface OverlayGeometry {
  windowW: number; collapsedH: number; expandedH: number;
  pillIdleW: number; pillActiveW: number;
  pillMarginIdle: number; pillMarginActive: number;
  dropdownH: number; wingW: number;
  floating: boolean;
  island: IslandMetrics;
}
export type OverlayActivity = 'idle' | 'recording' | 'processing';
export type OverlayPlacement = 'notch' | 'floating_pill';
/** How big the floating pill rests; Rust owns the pixels. `compact` is the
 * dot and waveform alone (Live Words off), `words` adds the two-line word
 * wheel (wider and taller) and is fixed for a whole dictation, `warning` is
 * the slim pill widened for a speech-model cue. */
export type OverlayPillSize = 'compact' | 'words' | 'warning';
/** The pill width the overlay asks for. The notch has none. A speech-model
 * warning beats words, which beat compact. `liveWordsConsented` is false until
 * the durable settings read completes, so the pill stays compact until then. */
export function overlayPillSize(
  placement: OverlayPlacement,
  modelWarning: boolean,
  liveWordsConsented: boolean,
): OverlayPillSize | undefined {
  if (placement !== 'floating_pill') return undefined;
  if (modelWarning) return 'warning';
  return liveWordsConsented ? 'words' : 'compact';
}
export type OverlayContent ='controls' | 'meeting_suggestion';
const NUMBER_KEYS = ['windowW', 'collapsedH', 'expandedH', 'pillIdleW', 'pillActiveW',
  'pillMarginIdle', 'pillMarginActive', 'dropdownH', 'wingW'] as const;
const ISLAND_KEYS = ['notchW', 'notchH', 'cornerIdle', 'cornerActive', 'pillW', 'pillH'] as const;
export function isOverlayGeometry(v: unknown): v is OverlayGeometry {
  if (typeof v !== 'object' || v === null) return false;
  const o = v as Record<string, unknown>;
  const island = o.island as Record<string, unknown> | null;
  return Object.keys(o).length === NUMBER_KEYS.length + 2
    && NUMBER_KEYS.every((k) => typeof o[k] === 'number' && Number.isFinite(o[k] as number))
    && typeof o.floating === 'boolean'
    && !!island && typeof island === 'object' && Object.keys(island).length === ISLAND_KEYS.length
    && ISLAND_KEYS.every((k) => typeof island[k] === 'number' && Number.isFinite(island[k] as number));
}
