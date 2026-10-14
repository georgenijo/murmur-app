export interface OverlayGeometry {
  windowW: number; collapsedH: number; expandedH: number;
  pillIdleW: number; pillActiveW: number;
  pillMarginIdle: number; pillMarginActive: number;
  dropdownH: number; wingW: number;
  floating: boolean;
}

export type OverlayContent = 'controls' | 'meeting_suggestion';

const NUMBER_KEYS = ['windowW', 'collapsedH', 'expandedH', 'pillIdleW', 'pillActiveW',
  'pillMarginIdle', 'pillMarginActive', 'dropdownH', 'wingW'] as const;

export function isOverlayGeometry(v: unknown): v is OverlayGeometry {
  if (typeof v !== 'object' || v === null) return false;
  const o = v as Record<string, unknown>;
  return Object.keys(o).length === NUMBER_KEYS.length + 1
    && NUMBER_KEYS.every((k) => typeof o[k] === 'number' && Number.isFinite(o[k] as number))
    && typeof o.floating === 'boolean';
}
