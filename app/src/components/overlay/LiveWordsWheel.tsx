import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useReducedMotion } from 'motion/react';
import { liveWordsTail } from '../../lib/liveWordsLine';
import {
  WHEEL_RENDERED_LINES, advanceWheel, newWheelCommitState, stepWheelSpring, wheelLineWidth,
  wheelPose, wheelTokens, type WheelLine,
} from '../../lib/liveWordsWheel';

/** Words measured per string; bounded so a long dictation cannot grow it. */
const MEASURE_CACHE_LIMIT = 4000;
/** Fallback when the webview cannot measure text (unit tests): about 6px
 * per code point at the pill's 11px. */
const FALLBACK_CHAR_W = 6;

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

/** The floating pill's Live Words on a picker wheel inside the slim pill.
 * The line being spoken sits in front, vertically centred; each finished line
 * rolls up and back out of the top over a cylinder, dimming and softening,
 * driven by a critically damped spring so it never jumps. At rest only the
 * front line shows. New words fade in from a slight blur. Finished lines are
 * committed and never roll back: a word only starts a new line once it has
 * existed for WHEEL_COMMIT_MS (see `advanceWheel`). `maxWidth` is the widest
 * line the pill can hold; `onFrontWidth` reports the front line's text width
 * (null before any words) so the pill can hug it. Under Reduce Motion there
 * is no roll, only a crossfade between lines. Without `hint` the wheel
 * takes no room before the first words (the fitted pill is then just the dot
 * and the waveform). */
export function LiveWordsWheel({ text, maxWidth, onFrontWidth, hint = true }: {
  text: string;
  maxWidth: number;
  onFrontWidth?: (width: number | null) => void;
  hint?: boolean;
}) {
  const wheel = useRef<HTMLDivElement>(null);
  const reduced = useReducedMotion() ?? false;
  const [measure, setMeasure] = useState<Measure | null>(null);
  const commit = useRef(newWheelCommitState());
  // Bumped when a held word becomes old enough to start a new line.
  const [tick, setTick] = useState(0);

  // Measure with the wheel's own font.
  useLayoutEffect(() => {
    const el = wheel.current;
    if (!el) return;
    const style = getComputedStyle(el);
    setMeasure(() => createMeasure(`${style.fontWeight} ${style.fontSize} ${style.fontFamily}`));
  }, []);

  const view = useMemo(() => {
    if (!measure) return null;
    const tokens = wheelTokens(text, measure, maxWidth);
    return advanceWheel(commit.current, tokens, performance.now(), measure, maxWidth);
    // `tick` re-evaluates held words without a new partial.
  }, [text, measure, maxWidth, tick]);

  useEffect(() => {
    if (view?.recheckInMs == null) return;
    const timer = setTimeout(() => setTick((n) => n + 1), view.recheckInMs + 1);
    return () => clearTimeout(timer);
  }, [view]);

  const lines = view?.lines ?? [];
  const newest = Math.max(0, lines.length - 1);
  const front = lines[newest] ?? [];
  const frontWidth = measure && front.length ? Math.round(wheelLineWidth(front, measure) * 2) / 2 : null;
  useLayoutEffect(() => { onFrontWidth?.(frontWidth); }, [onFrontWidth, frontWidth]);
  useLayoutEffect(() => () => { onFrontWidth?.(null); }, [onFrontWidth]);
  const first = Math.max(0, lines.length - WHEEL_RENDERED_LINES);
  const shown = lines.slice(first);
  const tail = liveWordsTail(text);

  return (
    <div className={!tail && !hint ? 'bezel-wheel bezel-wheel-idle' : 'bezel-wheel'} ref={wheel}
      data-motion={reduced ? 'reduce' : 'roll'} data-line={newest}>
      {/* Screen readers get the words once, as text; the wheel is decoration. */}
      <span className="sr-only" role="status" aria-live="polite" aria-label="Words recognized so far">{tail}</span>
      {!tail ? hint && (
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

/** Reduce Motion: one flat line. A new line fades in in place of the old
 * one; nothing moves or turns. */
function CrossfadeLines({ lines, newest }: { lines: WheelLine[]; newest: number }) {
  return (
    <div key={newest} className="bezel-wheel-line bezel-wheel-fade" data-front aria-hidden="true">
      <LineWords words={lines[newest] ?? []} animate={false} />
    </div>
  );
}
