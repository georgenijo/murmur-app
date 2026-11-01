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
export type OverlayContent = 'controls' | 'meeting_suggestion';
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
