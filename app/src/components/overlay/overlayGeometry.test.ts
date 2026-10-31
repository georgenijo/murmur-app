import { describe, expect, it } from 'vitest';
import fixture from './overlay-geometry.fixture.json';
import { isOverlayGeometry } from '../../lib/overlayGeometry';
import type { OverlayGeometry } from '../../lib/overlayGeometry';

const entries: Array<[string, OverlayGeometry]> = [
  ['notched', fixture.notched],
  ['external', fixture.external],
  ['fallback', fixture.fallback],
  ['belowRecording', fixture.belowRecording],
  ['expandedMeetingNotched', fixture.expandedMeetingNotched],
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
      windowW: 241, collapsedH: 72, expandedH: 136,
      pillIdleW: 185, pillActiveW: 241,
      pillMarginIdle: 28, pillMarginActive: 0,
      dropdownH: 64, wingW: 28, floating: false, island: fixture.notched.island,
    });
    expect(fixture.external).toEqual({
      windowW: 241, collapsedH: 70, expandedH: 134,
      pillIdleW: 185, pillActiveW: 241,
      pillMarginIdle: 28, pillMarginActive: 0,
      dropdownH: 64, wingW: 28, floating: false, island: fixture.external.island,
    });
    expect(fixture.fallback).toEqual({
      windowW: 241, collapsedH: 77, expandedH: 141,
      pillIdleW: 185, pillActiveW: 241,
      pillMarginIdle: 28, pillMarginActive: 0,
      dropdownH: 64, wingW: 28, floating: false, island: fixture.fallback.island,
    });
  });

  it('locks native island metrics and exact steady-state bounds', () => {
    expect(fixture.notched.island).toEqual({ notchW: 185, notchH: 32, pad: 7, inset: 7, topOffset: 6, notchGap: 10, spread: 28, plateIdleW: 30, plateRecordingW: 112, plateProcessingW: 76, plateH: 26, cornerIdle: 8, cornerActive: 18, innerRadius: 11 });
    expect([fixture.idleNotched.windowW, fixture.idleNotched.collapsedH]).toEqual([185, 32]);
    expect([fixture.belowIdle.windowW, fixture.belowRecording.windowW, fixture.belowProcessing.windowW]).toEqual([44, 126, 90]);
    for (const g of Object.values(fixture)) expect(isOverlayGeometry(g)).toBe(true);
    expect(isOverlayGeometry({ ...fixture.notched, island: { ...fixture.notched.island, extra: 1 } })).toBe(false);
  });

  it('rejects unilateral shape drift', () => {
    expect(isOverlayGeometry({ ...fixture.notched, extraField: 1 })).toBe(false);
  });
});
