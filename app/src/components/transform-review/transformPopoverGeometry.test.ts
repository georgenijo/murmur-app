import { describe, expect, it } from 'vitest';
import fixture from './transform-popover-geometry.fixture.json';

interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
  flipped: boolean;
}

interface Case {
  selectionBounds: { x: number; y: number; width: number; height: number } | null;
  screenFrame: { x: number; y: number; width: number; height: number };
  output: { compact: Box; expanded: Box };
}

const cases: Array<[string, Case]> = [
  ['anchored', fixture.anchored as Case],
  ['flippedAbove', fixture.flippedAbove as Case],
  ['clampedLeft', fixture.clampedLeft as Case],
  ['clampedRight', fixture.clampedRight as Case],
  ['centeredFallback', fixture.centeredFallback as Case],
  ['nonPrimaryDisplay', fixture.nonPrimaryDisplay as Case],
];

/**
 * Same fixture-mechanism as `overlay/overlayGeometry.test.ts`: Rust owns
 * `popover_geometry_for()` and asserts this exact JSON via
 * `commands::transform_popover::tests::matches_fixture`. This test doesn't
 * recompute the geometry (the frontend never does — Rust is the sole author)
 * — it only locks the checked-in shape/values and the invariants that must
 * hold for any valid geometry, so an unreviewed edit to either side's copy of
 * the fixture is caught by whichever test runs first in CI.
 */
describe('transform popover geometry contract fixture', () => {
  it.each(cases)('has a well-formed compact/expanded box (%s)', (_name, c) => {
    for (const box of [c.output.compact, c.output.expanded]) {
      expect(box.width).toBeGreaterThan(0);
      expect(box.height).toBeGreaterThan(0);
      expect(typeof box.flipped).toBe('boolean');
    }
  });

  it('expanded is never smaller than compact', () => {
    for (const [, c] of cases) {
      expect(c.output.expanded.width).toBeGreaterThanOrEqual(c.output.compact.width);
      expect(c.output.expanded.height).toBeGreaterThanOrEqual(c.output.compact.height);
    }
  });

  it('never overlaps the menu-bar/notch band (box y >= visible frame y)', () => {
    for (const [, c] of cases) {
      expect(c.output.compact.y).toBeGreaterThanOrEqual(c.screenFrame.y);
      expect(c.output.expanded.y).toBeGreaterThanOrEqual(c.screenFrame.y);
    }
  });

  it('clamps horizontally within the visible frame', () => {
    for (const [, c] of cases) {
      for (const box of [c.output.compact, c.output.expanded]) {
        expect(box.x).toBeGreaterThanOrEqual(c.screenFrame.x);
        expect(box.x + box.width).toBeLessThanOrEqual(c.screenFrame.x + c.screenFrame.width);
      }
    }
  });

  it('locks the anchored case values', () => {
    expect(fixture.anchored.output).toEqual({
      compact: { x: 460, y: 328, width: 320, height: 76, flipped: false },
      expanded: { x: 410, y: 328, width: 420, height: 220, flipped: false },
    });
  });

  it('locks the flipped-above case values', () => {
    expect(fixture.flippedAbove.output).toEqual({
      compact: { x: 460, y: 766, width: 320, height: 76, flipped: true },
      expanded: { x: 410, y: 622, width: 420, height: 220, flipped: true },
    });
  });

  it('locks the centered-fallback case values', () => {
    expect(fixture.centeredFallback.selectionBounds).toBeNull();
    expect(fixture.centeredFallback.output).toEqual({
      compact: { x: 560, y: 319.5, width: 320, height: 76, flipped: false },
      expanded: { x: 510, y: 247.5, width: 420, height: 220, flipped: false },
    });
  });

  it('locks the clamped-left case values', () => {
    expect(fixture.clampedLeft.output).toEqual({
      compact: { x: 0, y: 328, width: 320, height: 76, flipped: false },
      expanded: { x: 0, y: 328, width: 420, height: 220, flipped: false },
    });
  });

  it('locks the clamped-right case values', () => {
    expect(fixture.clampedRight.output).toEqual({
      compact: { x: 1120, y: 328, width: 320, height: 76, flipped: false },
      expanded: { x: 1020, y: 328, width: 420, height: 220, flipped: false },
    });
  });

  it('locks the non-primary-display case values (negative-origin screen frame)', () => {
    // Regression coverage for a secondary display placed to the left of the
    // primary one (negative x) with no menu bar/notch inset — the visible
    // frame's own x/y must flow straight through into the resolved box
    // instead of the popover assuming it always originates at (0, 0).
    expect(fixture.nonPrimaryDisplay.screenFrame.x).toBeLessThan(0);
    expect(fixture.nonPrimaryDisplay.output).toEqual({
      compact: { x: -1300, y: 528, width: 320, height: 76, flipped: false },
      expanded: { x: -1350, y: 528, width: 420, height: 220, flipped: false },
    });
  });
});
