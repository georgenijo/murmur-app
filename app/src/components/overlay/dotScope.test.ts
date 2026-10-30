import { describe, expect, it } from 'vitest';
import geometry from './overlay-geometry.fixture.json';
import { AutoLevel, DOT_SCOPE, DotScope, loudness, type ProcessingStyle } from './dotScope';
import { ISLAND_FADE, ISLAND_SPRING, Spring, springEasing, springSettleMs } from './spring';

function run(scope: DotScope, seconds: number, hz = 60, db = -38, reduced = false) {
  for (let i = 0; i < seconds * hz; i++) {
    scope.advance(1 / hz, 'recording', db, reduced);
    scope.sample(1 / hz, 'flow', reduced);
  }
  return scope;
}
const energy = (values: Float32Array) => values.reduce((sum, x) => sum + x, 0);

describe('voice island simulation', () => {
  it('locks the specified spatial and temporal constants', () => {
    expect(DOT_SCOPE).toEqual({ columns: 43, rows: 9, pitch: 2.6, height: 26,
      idleWidth: 30, recordingWidth: 112, processingWidth: 76,
      turns: 1.6, response: 6.5, release: 0.15, historySpeed: 150,
      phosphor: 0.6, attack: 0.04, roomFall: 0.3, roomRise: 6,
      minSpan: 16, noiseFloor: -60, step: 1 / 240, maxSteps: 12 });
    expect(ISLAND_SPRING).toEqual({ duration: 0.3, bounce: 0.15 });
    expect(ISLAND_FADE).toEqual({ duration: 0.3, bounce: 0 });
  });

  it('uses native plate geometry rather than a second production size contract', () => {
    const native = geometry.notched.island;
    expect([native.plateIdleW, native.plateRecordingW, native.plateProcessingW, native.plateH])
      .toEqual([DOT_SCOPE.idleWidth, DOT_SCOPE.recordingWidth, DOT_SCOPE.processingWidth, DOT_SCOPE.height]);
    const scope = run(new DotScope({ ...native, plateRecordingW: 100 }), 2);
    expect(scope.width.x).toBeCloseTo(100, 6);
    expect(scope.geometry.innerRadius).toBe(native.innerRadius);
  });

  it('has the same fixed-step state at 30, 60 and 120Hz, with bounded catch-up', () => {
    const scopes = [30, 60, 120].map(hz => run(new DotScope(), 2, hz));
    for (const scope of scopes) {
      expect(scope.steps).toBe(480);
      expect(scope.level).toBeCloseTo(scopes[0].level, 10);
      expect(scope.width.x).toBeCloseTo(scopes[0].width.x, 10);
    }
    const scope = scopes[0];
    expect(scope.advance(60, 'recording', -20, false)).toBe(12);
    expect(scope.advance(Number.NaN, 'recording', -20, false)).toBe(0);
    expect(scope.advance(-1, 'recording', -20, false)).toBe(0);
  });

  it('auto-levels a quiet voice above the floor, rejects invalid RMS, and follows room asymmetrically', () => {
    expect(loudness(0.01)).toBe(-40);
    for (const rms of [0, -1, NaN, Infinity]) expect(loudness(rms)).toBe(-120);
    const quiet = new AutoLevel();
    for (let i = 0; i < 240; i++) quiet.step(-120, 1 / 240);
    expect(quiet.step(-48, 0.04)).toBeGreaterThan(0.3);
    expect(quiet.step(-120, 1 / 240)).toBe(0);
    expect(quiet.peak).toBeGreaterThanOrEqual(Math.max(quiet.room, -60) + 16);
    const down = new AutoLevel(); const up = new AutoLevel();
    down.step(-75, 0.3); up.step(-35, 0.3);
    expect(-55 - down.room).toBeGreaterThan(up.room + 55);
    const attack = new AutoLevel(); attack.step(-10, 0.04);
    expect(attack.peak).toBeCloseTo(-10 - 20 / Math.E, 5);
  });

  it('reuses bounded storage, lights the centre, and distinguishes processing styles', () => {
    const scope = run(new DotScope(), 2);
    const dots = scope.dots;
    expect(dots.length).toBe(43 * 9);
    expect(scope.sample(1 / 60, 'flow', false)).toBe(dots);
    let centre = 0; let edge = 0;
    for (let r = 0; r < 9; r++) { centre += dots[r * 43 + 21]; edge += dots[r * 43]; }
    expect(centre).toBeGreaterThan(edge);
    const patterns = (['sweep', 'flow', 'pulse'] as ProcessingStyle[]).map(style => {
      const s = new DotScope(); s.advance(0.05, 'processing', -120, false);
      for (let i = 0; i < 60; i++) { s.advance(1 / 60, 'processing', -120, false); s.sample(1 / 60, style, false); }
      return Array.from(s.dots);
    });
    expect(patterns[0]).not.toEqual(patterns[1]); expect(patterns[1]).not.toEqual(patterns[2]);
    for (const power of patterns.flat()) { expect(Number.isFinite(power)).toBe(true); expect(power).toBeGreaterThanOrEqual(0); expect(power).toBeLessThanOrEqual(1); }
  });

  it('Done clears the centre before the edges, then becomes transparent', () => {
    const s = run(new DotScope(), 2);
    s.advance(0.05, 'done', -120, false); s.sample(0.05, 'flow', false);
    // Inspect at 100ms, before the wipe reaches the 28.6pt outside column.
    s.advance(0.05, 'done', -120, false); s.sample(0.05, 'flow', false);
    let centre = 0; let outside = 0;
    for (let r = 0; r < 9; r++) { centre += s.dots[r * 43 + 21]; outside += s.dots[r * 43 + 32]; }
    expect(centre).toBe(0); expect(outside).toBeGreaterThan(0);
    for (let i = 0; i < 60; i++) { s.advance(1 / 60, 'done', -120, false); s.sample(1 / 60, 'flow', false); }
    expect(energy(s.dots)).toBe(0); expect(s.opacity.x).toBeLessThan(0.002);
  });

  it('Reduce Motion has an invariant field and no width, scale, blur, travel or wipe animation', () => {
    const scope = run(new DotScope(), 1, 60, -20, true);
    const before = Array.from(scope.dots);
    run(scope, 1, 120, -55, true);
    expect(Array.from(scope.dots)).toEqual(before);
    expect(scope.width.x).toBe(112); expect(scope.scale.x).toBe(1); expect(scope.blur.x).toBe(0);
    scope.advance(0.05, 'processing', -120, true); const flow = Array.from(scope.sample(0.05, 'flow', true));
    scope.advance(0.05, 'processing', -120, true); expect(Array.from(scope.sample(0.05, 'pulse', true))).toEqual(flow);
  });
});

describe('closed-form island springs', () => {
  it('preserves position and velocity across retargets and settles to the target', () => {
    const spring = new Spring(30);
    spring.stepTo(112, ISLAND_SPRING, 0.08);
    const { x, v } = spring;
    spring.stepTo(76, ISLAND_SPRING, 0);
    expect(spring.x).toBe(x); expect(spring.v).toBe(v);
    spring.step(ISLAND_SPRING, 3);
    expect(spring.x).toBeCloseTo(76, 6); expect(spring.v).toBeCloseTo(0, 6);
  });

  it('uses the same sampled oscillator and settlement duration for CSS', () => {
    expect(springEasing(ISLAND_SPRING)).toMatch(/^linear\(/);
    expect(springSettleMs(ISLAND_SPRING)).toBeGreaterThanOrEqual(300);
    const fade = new Spring(0);
    for (let i = 0; i < 240; i++) { fade.stepTo(1, ISLAND_FADE, 1 / 240); expect(fade.x).toBeLessThanOrEqual(1); }
    expect(fade.x).toBeCloseTo(1, 3);
  });
});
