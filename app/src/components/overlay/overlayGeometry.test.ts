import { describe, expect, it } from 'vitest';
import fixture from './overlay-geometry.fixture.json';
import { isOverlayGeometry } from '../../lib/overlayGeometry';
import type { OverlayGeometry } from '../../lib/overlayGeometry';

const entries: Array<[string, OverlayGeometry]> = [
  ['notched', fixture.notched],
  ['fallback', fixture.fallback],
];

describe('overlay geometry contract fixture', () => {
  it('validates as OverlayGeometry', () => {
    expect(isOverlayGeometry(fixture.notched)).toBe(true);
    expect(isOverlayGeometry(fixture.fallback)).toBe(true);
  });

  it.each(entries)('holds the geometry invariants (%s)', (_name, g) => {
    expect(g.windowW).toBeGreaterThanOrEqual(g.pillActiveW + g.pillMarginActive);
    expect(g.windowW).toBeGreaterThanOrEqual(g.pillIdleW + g.pillMarginIdle);
    expect(g.expandedH).toBe(g.collapsedH + g.dropdownH);
    expect(g.pillActiveW).toBeGreaterThanOrEqual(g.pillIdleW);
  });

  it('locks the characterization values', () => {
    expect(fixture.notched).toEqual({
      windowW: 305, collapsedH: 32, expandedH: 76,
      pillIdleW: 213, pillActiveW: 305,
      pillMarginIdle: 46, pillMarginActive: 0,
      dropdownH: 44,
    });
    expect(fixture.fallback).toEqual({
      windowW: 200, collapsedH: 37, expandedH: 81,
      pillIdleW: 108, pillActiveW: 200,
      pillMarginIdle: 46, pillMarginActive: 0,
      dropdownH: 44,
    });
  });

  it('rejects unilateral shape drift', () => {
    expect(isOverlayGeometry({ ...fixture.notched, extraField: 1 })).toBe(false);
  });
});
