// Hex a07f994 dot_scope.rs auto-level and spring constants (MIT, Christopher
// Langton 2026); see THIRD_PARTY_NOTICES.md. The bounded canvas simulation is
// Murmur's implementation, not a port of the GPU/nearest-segment renderer.
import { Spring, ISLAND_SPRING, ISLAND_FADE } from './spring';
import type { IslandMetrics } from '../../lib/overlayGeometry';

export type DotScopeGeometry = Pick<IslandMetrics,
  'plateIdleW' | 'plateRecordingW' | 'plateProcessingW' | 'plateH' | 'innerRadius'>;

export const DOT_SCOPE = Object.freeze({
  columns: 43, rows: 9, pitch: 2.6, height: 26,
  idleWidth: 30, recordingWidth: 112, processingWidth: 76,
  turns: 1.6, response: 6.5, release: 0.15, historySpeed: 150,
  phosphor: 0.6, attack: 0.04, roomFall: 0.3, roomRise: 6,
  minSpan: 16, noiseFloor: -60, step: 1 / 240, maxSteps: 12,
});
export const DOT_ALPHAS = 24;
export type ScopeStage = 'idle' | 'recording' | 'processing' | 'done';
export type ProcessingStyle = 'sweep' | 'flow' | 'pulse';
const TAU = 2 * Math.PI;
const HISTORY = 512;
const follow = (x: number, target: number, seconds: number, dt: number) =>
  x + (target - x) * (1 - Math.exp(-dt / seconds));
export const clamp = (x: number, low = 0, high = 1) => Math.max(low, Math.min(high, x));
export function loudness(rms: number): number {
  return 20 * Math.log10(Math.max(Number.isFinite(rms) ? rms : 0, 1e-6));
}
export class AutoLevel {
  peak = -30;
  room = -55;
  step(db: number, dt: number): number {
    const x = clamp(Number.isFinite(db) ? db : -120, -120, 0);
    this.room = follow(this.room, x, x < this.room ? DOT_SCOPE.roomFall : DOT_SCOPE.roomRise, dt);
    const lowest = Math.max(this.room, DOT_SCOPE.noiseFloor) + DOT_SCOPE.minSpan;
    this.peak = Math.max(lowest, follow(this.peak, x > this.peak ? x : lowest,
      x > this.peak ? DOT_SCOPE.attack : 3, dt));
    const low = Math.max(Math.max(this.room, DOT_SCOPE.noiseFloor) + 5, this.peak - 24);
    return clamp((x - low) / Math.max(this.peak - low, 1));
  }
}

/** Fixed-step state is independent of refresh rate. No arrays/objects are
 * allocated by advance/sample; a stalled webview drops old time, not 240Hz work. */
export class DotScope {
  readonly auto = new AutoLevel();
  readonly width: Spring;
  readonly scale = new Spring(0.9);
  readonly opacity = new Spring(0);
  readonly blur = new Spring(3);
  readonly dots = new Float32Array(DOT_SCOPE.columns * DOT_SCOPE.rows);
  readonly buckets = new Uint8Array(this.dots.length);
  readonly radii = new Float32Array(this.dots.length);
  readonly paintOrder = new Uint16Array(this.dots.length);
  readonly bucketCounts = new Uint16Array(DOT_ALPHAS);
  readonly bucketOffsets = new Uint16Array(DOT_ALPHAS);
  private readonly history = new Float32Array(HISTORY);
  private readonly phosphor = new Float32Array(this.dots.length);
  private head = 0;
  private remainder = 0;
  private envelope = 0;
  private amplitude = 0;
  private velocity = 0;
  private elapsed = 0;
  private time = 0;
  private stage: ScopeStage = 'idle';
  level = 0;
  steps = 0;

  // Production passes the native contract. Defaults serve standalone numerical
  // tests only; no fallback window geometry is synthesized in the app.
  constructor(readonly geometry: DotScopeGeometry = {
    plateIdleW: DOT_SCOPE.idleWidth, plateRecordingW: DOT_SCOPE.recordingWidth,
    plateProcessingW: DOT_SCOPE.processingWidth, plateH: DOT_SCOPE.height, innerRadius: 11,
  }) {
    this.width = new Spring(geometry.plateIdleW);
  }

  advance(dt: number, stage: ScopeStage, db: number, reduced: boolean): number {
    if (stage !== this.stage) { this.stage = stage; this.elapsed = 0; }
    this.remainder = Math.min(this.remainder + Math.max(Number.isFinite(dt) ? dt : 0, 0),
      DOT_SCOPE.step * DOT_SCOPE.maxSteps);
    let steps = 0;
    while (this.remainder + 1e-10 >= DOT_SCOPE.step && steps < DOT_SCOPE.maxSteps) {
      this.remainder -= DOT_SCOPE.step;
      const h = DOT_SCOPE.step;
      this.level = stage === 'recording' ? this.auto.step(db, h) : 0;
      this.envelope = follow(this.envelope, this.level,
        this.level > this.envelope ? 0.022 : DOT_SCOPE.release, h);
      const omega = TAU * DOT_SCOPE.response;
      this.velocity += (-omega * omega * (this.amplitude - this.envelope)
        - 2 * 0.52 * omega * this.velocity) * h;
      this.amplitude += this.velocity * h;
      this.history[this.head] = clamp(this.amplitude);
      this.head = (this.head + 1) % HISTORY;
      const width = stage === 'recording' ? this.geometry.plateRecordingW
        : stage === 'processing' || stage === 'done' ? this.geometry.plateProcessingW : this.geometry.plateIdleW;
      const shown = stage === 'idle' ? 0 : stage === 'done' ? clamp(1 - this.elapsed / 0.4) : 1;
      if (reduced) {
        this.width.jump(width); this.scale.jump(1); this.blur.jump(0);
      } else {
        this.width.stepTo(width, ISLAND_SPRING, h);
        this.scale.stepTo(stage === 'idle' ? 0.9 : 1, ISLAND_SPRING, h);
        this.blur.stepTo(stage === 'idle' ? 3 : 0, ISLAND_FADE, h);
      }
      this.opacity.stepTo(shown, ISLAND_FADE, h);
      this.elapsed += h;
      this.time += h;
      steps++; this.steps++;
    }
    return steps;
  }

  visibilityAt(distance: number, reduced: boolean): number {
    const halfW = Math.max(this.width.x, 26) / 2;
    return this.stage === 'done' && !reduced
      ? clamp((distance - this.elapsed * (halfW + 12) / 0.3) / 6) : 1;
  }

  /** Two opposed 1.6-turn traces, outward-delayed amplitude, centre falloff,
   * and a time-based phosphor tail. Called once per display frame. */
  sample(dt: number, style: ProcessingStyle, reduced: boolean): Float32Array {
    const keep = Math.exp(-Math.min(Math.max(dt, 0), 0.05) / DOT_SCOPE.phosphor);
    const halfW = Math.max(this.width.x, 26) / 2;
    for (let c = 0; c < DOT_SCOPE.columns; c++) {
      const x = (c - 21) * DOT_SCOPE.pitch;
      const distance = Math.abs(x);
      const delay = distance / DOT_SCOPE.historySpeed / DOT_SCOPE.step;
      const past = this.head - 1 - Math.floor(delay);
      const a = (past + 2 * HISTORY) % HISTORY;
      const b = (a + HISTORY - 1) % HISTORY;
      const amplitude = reduced ? 0.35 : this.history[a] + (this.history[b] - this.history[a]) * (delay % 1);
      const edge = clamp((halfW - distance) / 14);
      const falloff = 1 - 0.8 * clamp(distance / halfW);
      const phase = TAU * DOT_SCOPE.turns * (x / DOT_SCOPE.recordingWidth + 0.5)
        - (reduced ? 0 : this.time * TAU * 0.6);
      const y = Math.sin(phase) * (0.7 + 3.1 * amplitude * falloff) * edge;
      let work = 0;
      if (this.stage === 'processing' || this.stage === 'done') {
        if (reduced) work = 0.4;
        else if (style === 'sweep') work = Math.exp(-(((x - Math.sin(this.time * TAU / 1.1) * halfW) / 7) ** 2));
        else if (style === 'flow') work = (0.5 + 0.5 * Math.cos(x / 8 - this.time * TAU / 1.1)) ** 4;
        else work = Math.exp(-(((distance - (this.elapsed * 46) % (halfW + 12)) / 7) ** 2));
      }
      const wipe = this.visibilityAt(distance, reduced);
      // The opposed traces and phosphor are symmetric about the centre row.
      // Evaluate the nearer trace once per row pair, retaining the exact
      // Gaussian (and Float32 phosphor rounding) rather than approximating it.
      for (let r = 0; r <= 4; r++) {
        const n = r * DOT_SCOPE.columns + c;
        const opposite = (DOT_SCOPE.rows - 1 - r) * DOT_SCOPE.columns + c;
        const dy = 4 - r;
        const trace = Math.exp(-(((dy - Math.abs(y)) / 0.68) ** 2));
        const power = edge * falloff * (this.stage === 'recording'
          ? trace * (0.2 + 0.8 * amplitude) : trace * (0.2 + 0.8 * work));
        this.phosphor[n] = reduced ? power : Math.max(power, this.phosphor[n] * keep) * wipe;
        this.dots[n] = this.phosphor[n];
        this.phosphor[opposite] = this.phosphor[n];
        this.dots[opposite] = this.phosphor[n];
      }
    }
    return this.dots;
  }
}
