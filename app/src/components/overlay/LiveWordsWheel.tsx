import { Fragment, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useReducedMotion } from 'motion/react';
import { liveWordsTail } from '../../lib/liveWordsLine';
import {
  WHEEL_RENDERED_LINES, stepWheelSpring, wheelPose, wrapWheelLines, type WheelLine,
} from '../../lib/liveWordsWheel';

/** Words measured per string; bounded so a long dictation cannot grow it. */
const MEASURE_CACHE_LIMIT = 4000;
/** Fallback when the webview cannot measure text (unit tests): about 7.5px
 * per code point at the wheel's 15px. */
const FALLBACK_CHAR_W = 7.5;
const FALLBACK_LINE_W = 225;
/** Lines are inset this far from each side of the wheel, inside the band. */
const LINE_INSET = 6;

type Measure = (s: string) => number;

function createMeasure(font: string): Measure {
  const cache = new Map<string, number>();
  let ctx: OffscreenCanvasRenderingContext2D | null = null;
  try {
    ctx = typeof OffscreenCanvas === 'undefined' ? null : new OffscreenCanvas(1, 1).getContext('2d');
    if (ctx) ctx.font = font;
  } catch { ctx = null; }
  return (s) => {
    const hit = cache.get(s);
    if (hit !== undefined) return hit;
    const w = ctx ? ctx.measureText(s).width : Array.from(s).length * FALLBACK_CHAR_W;
    if (cache.size >= MEASURE_CACHE_LIMIT) cache.clear();
    cache.set(s, w);
    return w;
  };
}

/** The floating pill's Live Words: a two-line picker wheel. The line being
 * spoken sits in front, a little below centre; each finished line rolls up and
 * back over a cylinder, dimming and softening, driven by a critically damped
 * spring so it never jumps. New words fade in from a slight blur. The pill's
 * size is native and fixed: long text becomes more lines, never a wider pill.
 * Under Reduce Motion there is no roll, only a crossfade between lines. */
export function LiveWordsWheel({ text }: { text: string }) {
  const wheel = useRef<HTMLDivElement>(null);
  const reduced = useReducedMotion() ?? false;
  const [frame, setFrame] = useState<{ width: number; measure: Measure } | null>(null);

  // Measure with the wheel's own font, and re-wrap if its width changes.
  useLayoutEffect(() => {
    const el = wheel.current;
    if (!el) return;
    const style = getComputedStyle(el);
    const measure = createMeasure(`${style.fontWeight} ${style.fontSize} ${style.fontFamily}`);
    const read = () => {
      const width = el.clientWidth ? el.clientWidth - 2 * LINE_INSET : FALLBACK_LINE_W;
      setFrame((prev) => prev && prev.width === width ? prev : { width, measure });
    };
    read();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(read);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const lines = useMemo<WheelLine[]>(
    () => frame ? wrapWheelLines(text, frame.measure, frame.width) : [],
    [text, frame],
  );
  const newest = Math.max(0, lines.length - 1);
  const first = Math.max(0, lines.length - WHEEL_RENDERED_LINES);
  const shown = lines.slice(first);
  const tail = liveWordsTail(text);

  return (
    <div className="bezel-wheel" ref={wheel} data-motion={reduced ? 'reduce' : 'roll'}>
      {/* Screen readers get the words once, as text; the wheel is decoration. */}
      <span className="sr-only" role="status" aria-live="polite" aria-label="Words recognized so far">{tail}</span>
      <div className="bezel-wheel-band" aria-hidden="true" />
      {!tail ? (
        <div className="bezel-wheel-line bezel-wheel-hint" data-front aria-hidden="true">Listening…</div>
      ) : !lines.length ? null : reduced ? (
        <CrossfadeLines lines={lines} newest={newest} />
      ) : (
        <RollingLines lines={shown} first={first} newest={newest} />
      )}
    </div>
  );
}

function LineWords({ words, animate }: { words: WheelLine; animate: boolean }) {
  // Keyed by position: a new word mounts (and fades in); a word the model
  // revises in place just updates.
  return <>{words.map((word, i) => (
    <Fragment key={i}>
      {i > 0 && ' '}
      <span className={animate ? 'bezel-wheel-word bezel-wheel-word-in' : 'bezel-wheel-word'}>{word}</span>
    </Fragment>
  ))}</>;
}

function RollingLines({ lines, first, newest }: { lines: WheelLine[]; first: number; newest: number }) {
  const els = useRef(new Map<number, HTMLDivElement>());
  const spring = useRef<{ pos: number; vel: number; target: number; raf: number; last: number } | null>(null);

  useLayoutEffect(() => {
    const draw = (pos: number) => {
      for (const [index, el] of els.current) {
        const pose = wheelPose(index - pos);
        el.style.visibility = pose.visible ? 'visible' : 'hidden';
        el.style.transform = pose.transform;
        el.style.opacity = pose.opacity.toFixed(3);
        el.style.filter = pose.blur > 0.05 ? `blur(${pose.blur.toFixed(2)}px)` : 'none';
        el.toggleAttribute('data-front', Math.round(pos) === index);
      }
    };
    const s = spring.current ??= { pos: newest, vel: 0, target: newest, raf: 0, last: 0 };
    s.target = newest;
    // Fast speech can add lines faster than the roll: never trail by more
    // than the lines still in the DOM.
    s.pos = Math.max(s.pos, newest - (WHEEL_RENDERED_LINES - 1));
    draw(s.pos);
    if (s.raf || (Math.abs(s.target - s.pos) < 0.001 && Math.abs(s.vel) < 0.001)) return;
    const tick = (now: number) => {
      const dt = s.last ? Math.min(0.05, (now - s.last) / 1000) : 1 / 60;
      s.last = now;
      [s.pos, s.vel] = stepWheelSpring(s.pos, s.vel, s.target, dt);
      if (Math.abs(s.target - s.pos) < 0.001 && Math.abs(s.vel) < 0.001) {
        s.pos = s.target; s.vel = 0; s.raf = 0; s.last = 0;
        draw(s.pos);
        return;
      }
      draw(s.pos);
      s.raf = requestAnimationFrame(tick);
    };
    s.raf = requestAnimationFrame(tick);
  });

  useLayoutEffect(() => () => {
    const s = spring.current;
    if (!s) return;
    if (s.raf) cancelAnimationFrame(s.raf);
    s.raf = 0; s.last = 0;
  }, []);

  return <>{lines.map((words, i) => {
    const index = first + i;
    return (
      <div key={index} className="bezel-wheel-line" aria-hidden="true"
        ref={(el) => { if (el) els.current.set(index, el); else els.current.delete(index); }}>
        <LineWords words={words} animate />
      </div>
    );
  })}</>;
}

/** Reduce Motion: two flat slots, the previous line dimmed above the front
 * one. A new line fades in where it lands; nothing moves or turns. */
function CrossfadeLines({ lines, newest }: { lines: WheelLine[]; newest: number }) {
  const previous = newest > 0 ? lines[newest - 1] : null;
  return <>
    {previous && (
      <div key={`p${newest - 1}`} className="bezel-wheel-line bezel-wheel-fade" data-slot="previous" aria-hidden="true">
        <LineWords words={previous} animate={false} />
      </div>
    )}
    <div key={`f${newest}`} className="bezel-wheel-line bezel-wheel-fade" data-slot="front" data-front aria-hidden="true">
      <LineWords words={lines[newest] ?? []} animate={false} />
    </div>
  </>;
}
