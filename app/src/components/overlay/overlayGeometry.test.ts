import { describe, expect, it } from 'vitest';
import fixture from './overlay-geometry.fixture.json';
import { isOverlayGeometry } from '../../lib/overlayGeometry';
import type { OverlayGeometry } from '../../lib/overlayGeometry';

const entries: Array<[string, OverlayGeometry]> = [
  ['notched', fixture.notched],
  ['external', fixture.external],
  ['fallback', fixture.fallback],
];

describe('overlay geometry contract fixture', () => {
  it('validates as OverlayGeometry', () => {
    expect(isOverlayGeometry(fixture.notched)).toBe(true);
    expect(isOverlayGeometry(fixture.external)).toBe(true);
    expect(isOverlayGeometry(fixture.fallback)).toBe(true);
  });

  it.each(entries)('holds the geometry invariants (%s)', (_name, g) => {
    expect(g.windowW).toBeGreaterThanOrEqual(g.pillActiveW + g.pillMarginActive);
    expect(g.windowW).toBeGreaterThanOrEqual(g.pillIdleW + g.pillMarginIdle);
    expect(g.expandedH).toBe(g.collapsedH + g.dropdownH);
    expect(g.pillActiveW).toBeGreaterThanOrEqual(g.pillIdleW);
    expect(g.wingW).toBeGreaterThan(0);
    expect(g.windowW).toBeGreaterThanOrEqual(2 * g.wingW);
  });

  it('locks the characterization values', () => {
    expect(fixture.notched).toEqual({
      windowW: 257, collapsedH: 32, expandedH: 96,
      pillIdleW: 221, pillActiveW: 257,
      pillMarginIdle: 0, pillMarginActive: 0,
      dropdownH: 64, wingW: 36, floating: false,
    });
    expect(fixture.external).toEqual({
      windowW: 280, collapsedH: 36, expandedH: 100,
      pillIdleW: 280, pillActiveW: 280,
      pillMarginIdle: 0, pillMarginActive: 0,
      dropdownH: 64, wingW: 36, floating: true,
    });
    expect(fixture.fallback).toEqual({
      windowW: 280, collapsedH: 36, expandedH: 100,
      pillIdleW: 280, pillActiveW: 280,
      pillMarginIdle: 0, pillMarginActive: 0,
      dropdownH: 64, wingW: 36, floating: true,
    });
  });

  it('rejects unilateral shape drift', () => {
    expect(isOverlayGeometry({ ...fixture.notched, extraField: 1 })).toBe(false);
  });
});
