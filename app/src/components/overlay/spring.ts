// Frame-rate independent spring physics for the notch island and its dot
// field. Ported from Hex `src/spring.rs` @ a07f994 (MIT, Copyright (c) 2026
// Christopher Langton); see THIRD_PARTY_NOTICES.md.
//
// A spec is SwiftUI's mapping: `ω = 2π / duration`, `ζ = 1 − bounce`. The
// step is the closed-form damped oscillator, so a trajectory is identical at
// 60 Hz, 120 Hz, or the dot scope's fixed 240 Hz step.

const CRITICAL_DAMPING_TOLERANCE = 1e-4;

export interface SpringSpec {
  /** Seconds. */
  duration: number;
  /** 0 is critically damped; positive overshoots. */
  bounce: number;
}

export function spec(duration: number, bounce: number): SpringSpec {
  return { duration, bounce };
}

export function omega(s: SpringSpec): number {
  return (2 * Math.PI) / Math.max(s.duration, 0.01);
}

export function zeta(s: SpringSpec): number {
  return s.bounce >= 0 ? Math.max(1 - s.bounce, 0) : 1 / Math.max(1 + s.bounce, 0.01);
}

/** Width and scale: Hex's `transition` 0.3 s with `bounce` 0.15. */
export const ISLAND_SPRING = spec(0.3, 0.15);
/** Opacity and blur: the same duration without overshoot. */
export const ISLAND_FADE = spec(0.3, 0);
/** The opacity-only crossfade kept when Reduce Motion is on. */
export const REDUCED_FADE = spec(0.16, 0);

export class Spring {
  x: number;
  v = 0;
  target: number;

  constructor(x: number) {
    this.x = x;
    this.target = x;
  }

  /** Snaps to `x` and clears velocity (first paint, Reduce Motion). */
  jump(x: number): void {
    this.x = x;
    this.v = 0;
    this.target = x;
  }

  stepTo(target: number, s: SpringSpec, dt: number): void {
    this.target = target;
    this.step(s, dt);
  }

  step(s: SpringSpec, dt: number): void {
    if (!(dt > 0) || !Number.isFinite(dt)) return;
    const w = omega(s);
    const z = zeta(s);
    const x0 = this.x - this.target;
    const v0 = this.v;
    if (z < 1 - CRITICAL_DAMPING_TOLERANCE) {
      const decay = z * w;
      const wd = w * Math.sqrt(1 - z * z);
      const e = Math.exp(-decay * dt);
      const sin = Math.sin(wd * dt);
      const cos = Math.cos(wd * dt);
      const sinOverD = sin / wd;
      this.x = this.target + e * (x0 * (cos + decay * sinOverD) + v0 * sinOverD);
      this.v = e * (-x0 * w * w * sinOverD + v0 * (cos - decay * sinOverD));
    } else if (z > 1 + CRITICAL_DAMPING_TOLERANCE) {
      const root = Math.sqrt(z * z - 1);
      const sum = z + root;
      const slow = -w / sum;
      const fast = -w * sum;
      const denom = slow - fast;
      const se = Math.exp(slow * dt);
      const fe = Math.exp(fast * dt);
      const p00 = (-fast * se + slow * fe) / denom;
      const p01 = (se - fe) / denom;
      const p10 = (slow * fast * (fe - se)) / denom;
      const p11 = (slow * se - fast * fe) / denom;
      this.x = this.target + p00 * x0 + p01 * v0;
      this.v = p10 * x0 + p11 * v0;
    } else {
      const e = Math.exp(-w * dt);
      const c = v0 + w * x0;
      this.x = this.target + (x0 + c * dt) * e;
      this.v = (v0 - w * c * dt) * e;
    }
  }
}

/**
 * How long the spring takes from rest at 0 to stay within `tolerance` of 1,
 * in milliseconds. CSS transitions driven by {@link springEasing} last this
 * long, and timers that wait for them (the native shrink) derive from it.
 */
export function springSettleMs(s: SpringSpec, tolerance = 0.002): number {
  const spring = new Spring(0);
  spring.target = 1;
  const h = 1 / 1000;
  let settledAt = 0;
  for (let i = 1; i <= 3000; i += 1) {
    spring.step(s, h);
    if (Math.abs(spring.x - 1) > tolerance || Math.abs(spring.v) > tolerance * 10) settledAt = i;
  }
  return settledAt;
}

/**
 * The spring from 0 to 1 as a CSS `linear()` easing over
 * {@link springSettleMs}: the browser interpolates the sampled curve, so a
 * width or height transition overshoots exactly as the dot field's own
 * spring does.
 */
export function springEasing(s: SpringSpec, samples = 32): string {
  const durationS = springSettleMs(s) / 1000;
  const spring = new Spring(0);
  spring.target = 1;
  const points: string[] = ['0'];
  for (let i = 1; i <= samples; i += 1) {
    spring.step(s, durationS / samples);
    points.push(i === samples ? '1' : String(Math.round(spring.x * 10000) / 10000));
  }
  return `linear(${points.join(', ')})`;
}
