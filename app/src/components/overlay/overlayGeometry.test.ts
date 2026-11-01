import { describe, expect, it } from 'vitest';
import fixture from './overlay-geometry.fixture.json';
import { isOverlayGeometry } from '../../lib/overlayGeometry';
import type { OverlayGeometry } from '../../lib/overlayGeometry';

const entries: Array<[string, OverlayGeometry]> = [
  ['notched', fixture.notched],
  ['external', fixture.external],
  ['fallback', fixture.fallback],
  ['pillRecording', fixture.pillRecording],
  ['pillExpanded', fixture.pillExpanded],
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
      windowW: 241, collapsedH: 32, expandedH: 96,
      pillIdleW: 185, pillActiveW: 241,
      pillMarginIdle: 28, pillMarginActive: 0,
      dropdownH: 64, wingW: 28, floating: false, island: fixture.notched.island,
    });
    expect(fixture.external).toEqual({
      windowW: 241, collapsedH: 30, expandedH: 94,
      pillIdleW: 185, pillActiveW: 241,
      pillMarginIdle: 28, pillMarginActive: 0,
      dropdownH: 64, wingW: 28, floating: false, island: fixture.external.island,
    });
    expect(fixture.fallback).toEqual({
      windowW: 241, collapsedH: 37, expandedH: 101,
      pillIdleW: 185, pillActiveW: 241,
      pillMarginIdle: 28, pillMarginActive: 0,
      dropdownH: 64, wingW: 28, floating: false, island: fixture.fallback.island,
    });
  });

  it('locks native island metrics and exact steady-state bounds', () => {
    expect(fixture.notched.island).toEqual({ notchW: 185, notchH: 32, cornerIdle: 8, cornerActive: 12, pillW: 112, pillH: 34 });
    // The notch wings stay inside the menu bar for every activity.
    expect([fixture.idleNotched.windowW, fixture.idleNotched.collapsedH]).toEqual([185, 32]);
    expect([fixture.recordingNotched.windowW, fixture.recordingNotched.collapsedH]).toEqual([241, 32]);
    // The floating pill never resizes between activities.
    for (const g of [fixture.pillIdle, fixture.pillRecording, fixture.pillProcessing]) {
      expect([g.windowW, g.collapsedH, g.floating]).toEqual([112, 34, true]);
    }
    for (const g of Object.values(fixture)) expect(isOverlayGeometry(g)).toBe(true);
    expect(isOverlayGeometry({ ...fixture.notched, island: { ...fixture.notched.island, extra: 1 } })).toBe(false);
  });

  it('rejects unilateral shape drift', () => {
    expect(isOverlayGeometry({ ...fixture.notched, extraField: 1 })).toBe(false);
  });
});
