import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { DOT_SCOPE, DOT_ALPHAS, DotScope, loudness, clamp, type DotScopeGeometry, type ScopeStage, type ProcessingStyle } from './dotScope';

const COLORS = ['rgb(222, 215, 201)', 'rgb(200, 220, 248)'];
const BLUR_FILTERS = Array.from({ length: 31 }, (_, i) => i ? `blur(${i / 10}px)` : 'none');
const COLUMN_X = Float64Array.from({ length: DOT_SCOPE.columns }, (_, c) => (c - 21) * DOT_SCOPE.pitch);
const ROW_Y = Float64Array.from({ length: DOT_SCOPE.rows }, (_, r) => (r - 4) * DOT_SCOPE.pitch);
const ROW_FLOOR = Float64Array.from(ROW_Y, y => 0.07 * (1 - 0.7 * Math.abs(y) / 13));

/** Canvas paths are batched by intensity; no React state or native RPC per frame.
 * The canvas is decorative: the existing priority-resolved status owns meaning. */
export function paintDots(ctx: CanvasRenderingContext2D, scope: DotScope, dt: number,
  style: ProcessingStyle, reduced: boolean) {
  const dots = scope.sample(dt, style, reduced);
  const g = scope.geometry;
  ctx.clearRect(0, 0, g.plateRecordingW, g.plateH);
  ctx.save();
  ctx.translate(g.plateRecordingW / 2, g.plateH / 2);
  ctx.scale(scope.scale.x, scope.scale.x);
  ctx.beginPath();
  ctx.roundRect(-scope.width.x / 2, -g.plateH / 2, scope.width.x, g.plateH, g.innerRadius);
  ctx.clip();
  ctx.fillStyle = COLORS[style === 'sweep' ? 1 : 0];
  // Compute nonlinear light/radius once, then batch paths by quantized alpha.
  // All scratch storage belongs to the mounted scope, not the display frame.
  scope.bucketCounts.fill(0);
  const opacity = clamp(scope.opacity.x);
  for (let c = 0; c < DOT_SCOPE.columns; c++) {
    const distance = Math.abs(COLUMN_X[c]);
    const mask = clamp((scope.width.x / 2 - distance - 1.4) / 3.2);
    const visibility = scope.visibilityAt(distance, reduced);
    // sample() maintains mirrored rows; share their nonlinear conversion too.
    for (let r = 0; r <= 4; r++) {
      const n = r * DOT_SCOPE.columns + c;
      const opposite = (DOT_SCOPE.rows - 1 - r) * DOT_SCOPE.columns + c;
      const floor = ROW_FLOOR[r];
      const light = dots[n];
      const alpha = (floor + (0.96 - floor) * light ** 0.9) * mask * opacity * visibility;
      const bucket = Math.round(alpha * (DOT_ALPHAS - 1));
      scope.buckets[n] = scope.buckets[opposite] = bucket;
      scope.bucketCounts[bucket] += n === opposite ? 1 : 2;
      scope.radii[n] = (0.25 + 0.17 * light ** 0.6) * DOT_SCOPE.pitch;
      scope.radii[opposite] = scope.radii[n];
    }
  }
  // Stable counting sort visits each dot once instead of rescanning the full
  // grid for every alpha. Keep row order within each path and skip empty fills.
  let total = 0;
  for (let bucket = 1; bucket < DOT_ALPHAS; bucket++) {
    scope.bucketOffsets[bucket] = total;
    total += scope.bucketCounts[bucket];
  }
  for (let n = 0; n < dots.length; n++) {
    const bucket = scope.buckets[n];
    if (bucket) scope.paintOrder[scope.bucketOffsets[bucket]++] = n;
  }
  let start = 0;
  for (let bucket = 1; bucket < DOT_ALPHAS; bucket++) {
    const end = scope.bucketOffsets[bucket];
    if (start === end) continue;
    ctx.beginPath();
    for (let i = start; i < end; i++) {
      const n = scope.paintOrder[i];
      const x = COLUMN_X[n % DOT_SCOPE.columns];
      const y = ROW_Y[Math.floor(n / DOT_SCOPE.columns)];
      const radius = scope.radii[n];
      ctx.moveTo(x + radius, y);
      ctx.arc(x, y, radius, 0, Math.PI * 2);
    }
    ctx.globalAlpha = bucket / (DOT_ALPHAS - 1);
    ctx.fill();
    start = end;
  }
  ctx.restore();
}

export function DotField({ stage, processingStyle = 'flow', fixtureAt, geometry }: {
  stage: ScopeStage; processingStyle?: ProcessingStyle; geometry?: DotScopeGeometry;
  /** Only the standalone fixture passes a fixed simulation time. No debug IPC. */
  fixtureAt?: number;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const scopeRef = useRef<DotScope | null>(null);
  if (!scopeRef.current) scopeRef.current = new DotScope(geometry);
  const level = useRef({ rms: 0, at: 0 });
  const nativeVisibleRef = useRef(true);
  const restartRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    if (fixtureAt !== undefined) return;
    let disposed = false;
    const stop = listen<number>('audio-level', ({ payload }) => {
      if (!disposed && Number.isFinite(payload)) {
        level.current.rms = Math.max(0, payload); level.current.at = performance.now();
      }
    }).catch(() => () => {});
    return () => { disposed = true; void stop.then((fn) => fn()); };
  }, [fixtureAt]);

  // Visibility belongs to the mounted native window, not to its current
  // recording stage or processing style. Keep one subscription across them.
  useEffect(() => {
    if (fixtureAt !== undefined) return;
    let disposed = false;
    const stop = listen<boolean>('overlay-visible-changed', ({ payload }) => {
      if (disposed) return;
      nativeVisibleRef.current = payload;
      restartRef.current?.();
    }).catch(() => () => {});
    return () => { disposed = true; void stop.then((fn) => fn()).catch(() => {}); };
  }, [fixtureAt]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext('2d');
    const scope = scopeRef.current!;
    if (!canvas || !ctx) return;
    const syncResolution = () => {
      const ratio = window.devicePixelRatio || 1;
      const width = Math.ceil(scope.geometry.plateRecordingW * ratio);
      const height = Math.ceil(scope.geometry.plateH * ratio);
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width; canvas.height = height;
      }
      ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    };
    syncResolution();
    const media = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    let reduced = media?.matches ?? false;
    let visible = !document.hidden;
    let disposed = false;
    let raf = 0;
    let last = 0;
    let staticDrawn = false;
    const draw = (now: number) => {
      raf = 0;
      if (disposed || !visible || !nativeVisibleRef.current) return;
      const dt = last ? Math.min((now - last) / 1000, 0.05) : 1 / 60;
      last = now;
      const db = now - level.current.at < 150 ? loudness(level.current.rms) : -120;
      scope.advance(dt, stage, db, reduced);
      canvas.style.filter = BLUR_FILTERS[Math.round(clamp(scope.blur.x, 0, 3) * 10)];
      paintDots(ctx, scope, dt, processingStyle, reduced);
      // Reduced Motion is fade only: settle once, then stop even while recording.
      // No idle loop; a new stage/visibility/media change wakes it explicitly.
      if (reduced) staticDrawn = scope.opacity.x > 0.998 || stage === 'idle' && scope.opacity.x < 0.002;
      if (stage !== 'idle' && stage !== 'done' && !staticDrawn
        || stage === 'idle' && scope.opacity.x > 0.002
        || stage === 'done' && scope.opacity.x > 0.002) raf = requestAnimationFrame(draw);
    };
    const restart = () => {
      cancelAnimationFrame(raf); raf = 0; last = 0; staticDrawn = false;
      if (visible && nativeVisibleRef.current) raf = requestAnimationFrame(draw);
    };
    const onVisibility = () => { visible = !document.hidden; restart(); };
    const onMotion = () => { reduced = media?.matches ?? false; restart(); };
    const onResize = () => { syncResolution(); restart(); };
    if (fixtureAt !== undefined) {
      const fixed = new DotScope(scope.geometry);
      for (let t = 0; t < fixtureAt; t += 1 / 60) {
        fixed.advance(1 / 60, stage === 'done' ? 'processing' : stage, -34 + 5 * Math.sin(t * 23), reduced);
        fixed.sample(1 / 60, processingStyle, reduced);
      }
      if (stage === 'done') {
        for (let t = 0; t < 0.12; t += 1 / 60) { fixed.advance(1 / 60, 'done', -120, reduced); fixed.sample(1 / 60, processingStyle, reduced); }
      }
      paintDots(ctx, fixed, 1 / 60, processingStyle, reduced);
      return;
    }
    document.addEventListener('visibilitychange', onVisibility);
    window.addEventListener('resize', onResize);
    media?.addEventListener('change', onMotion);
    restartRef.current = restart;
    restart();
    return () => {
      disposed = true; cancelAnimationFrame(raf);
      document.removeEventListener('visibilitychange', onVisibility);
      window.removeEventListener('resize', onResize);
      media?.removeEventListener('change', onMotion);
      if (restartRef.current === restart) restartRef.current = null;
    };
  }, [stage, processingStyle, fixtureAt, geometry]);

  return <canvas ref={canvasRef} className="overlay-dot-field" aria-hidden="true"
    data-columns={DOT_SCOPE.columns} data-rows={DOT_SCOPE.rows} data-pitch={DOT_SCOPE.pitch}
    style={{ width: scopeRef.current.geometry.plateRecordingW, height: scopeRef.current.geometry.plateH }} />;
}
