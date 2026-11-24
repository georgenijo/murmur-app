import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { OverlayIsland } from '../components/overlay/OverlayIsland';
import { OverlayPill } from '../components/overlay/OverlayPill';
import { OverlayDropdown } from '../components/overlay/OverlayDropdown';
import { OverlayMeetingSuggestion } from '../components/overlay/OverlayMeetingSuggestion';
import { DictationPreviewCard } from '../components/dictation-preview/DictationPreviewApp';
import { deriveVisual } from '../components/overlay/deriveVisual';
import geometry from '../components/overlay/overlay-geometry.fixture.json';
import type { OverlayGeometry } from '../lib/overlayGeometry';
import { overlayPillFitWidth } from '../lib/overlayPillFit';
import type { DictationStatus } from '../lib/types';

const noop = () => {};
const variants = ['idle', 'recording', 'processing', 'cancelled', 'miss', 'microphone', 'delivery', 'expanded', 'meeting', 'words', 'warning'] as const;
type Variant = typeof variants[number];
/** Floating-pill-only rows: the live words wheel, and the widened warning. */
const PILL_ONLY: readonly Variant[] = ['words', 'warning'];
/** Synthetic words for the `words` row; `?text=` swaps in a long or CJK line. */
const SYNTHETIC_WORDS = 'And it did not show up on the pill, so I think we can make it more minimal';

/** `?stream=<ms>` reveals the words one at a time, like partials arriving, so
 * the wheel can be watched rolling. `?revise=<word>` then imitates a pause:
 * every 520ms the model re-guesses its tail, alternately adding `<word>` and
 * dropping it again, six times. `window.__wheelStream` reports progress. */
function useStreamedWords(text: string | null): string | null {
  const query = new URLSearchParams(location.search);
  const ms = Number(query.get('stream')) || 0;
  const revise = query.get('revise');
  const words = text?.split(' ') ?? [];
  const [count, setCount] = useState(ms ? 0 : words.length);
  const [guess, setGuess] = useState(0);
  useEffect(() => {
    const streamed = count >= words.length;
    const done = streamed && (!revise || guess >= 6);
    (window as unknown as { __wheelStream?: string }).__wheelStream = done ? 'done' : 'streaming';
    if (done || (!ms && !revise)) return;
    const timer = streamed
      ? setTimeout(() => setGuess((n) => n + 1), 520)
      : setTimeout(() => setCount((n) => n + 1), ms);
    return () => clearTimeout(timer);
  }, [ms, revise, count, guess, words.length]);
  if (text === null) return null;
  const shown = words.slice(0, count);
  return (revise && guess % 2 === 1 ? [...shown, revise] : shown).join(' ');
}
const controlsStates = ['idle', 'starting', 'still-connecting', 'recording', 'recovering'] as const;
type ControlsState = typeof controlsStates[number];
/** Fixed synthetic levels so recording screenshots are deterministic. */
const SYNTHETIC_LEVELS = [0.3, 0.55, 0.8, 1, 0.7, 0.45, 0.6, 0.9, 0.5, 0.35, 0.75, 0.95, 0.65, 0.4, 0.85, 0.55, 0.3, 0.2];

function geometryKey(variant: Variant, pill: boolean, external: boolean): keyof typeof geometry {
  if (variant === 'meeting') return pill ? 'pillMeetingSuggestion' : 'expandedMeetingNotched';
  if (variant === 'expanded') return pill ? 'pillExpanded' : 'expandedNotched';
  if (variant === 'words') return 'pillWords';
  if (variant === 'warning') return 'pillWarning';
  const activity = variant === 'idle' ? 'idle' : variant === 'recording' ? 'recording' : 'processing';
  if (pill) return activity === 'idle' ? 'pillIdle' : activity === 'recording' ? 'pillRecording' : 'pillProcessing';
  return `${activity}${external ? 'External' : 'Notched'}`;
}

function FixtureRow({ variant, pill, external, fallback, meetingOnly, controlsState }: {
  variant: Variant; pill: boolean; external: boolean; fallback: boolean; meetingOnly: boolean;
  controlsState: ControlsState;
}) {
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);
  const meeting = variant === 'meeting';
  const expanded = meeting || variant === 'expanded';
  const idle = variant === 'idle';
  const recording = variant === 'recording';
  const g: OverlayGeometry = variant === 'expanded' && !pill && (external || fallback)
    ? geometry[fallback ? 'fallback' : 'external']
    : geometry[geometryKey(variant, pill, external)];
  const status: DictationStatus = variant === 'expanded'
    ? controlsState === 'still-connecting' ? 'starting' : controlsState
    : recording || variant === 'words' || ['cancelled', 'miss', 'microphone'].includes(variant)
      ? 'recording' : variant === 'processing' ? 'processing' : 'idle';
  const visual = deriveVisual(status, variant === 'cancelled', variant === 'miss', false, variant === 'microphone' ? 'chooseMicrophone' : null, false, false, 'idle', variant === 'delivery',
    variant === 'warning' ? 'incomplete' : null);
  const liveWords = useStreamedWords(variant === 'words' ? new URLSearchParams(location.search).get('text') ?? SYNTHETIC_WORDS : null);
  // The words row hugs its front line, as OverlayWidget does.
  const [frontW, setFrontW] = useState<number | null>(null);
  const fitW = variant === 'words' ? overlayPillFitWidth(true, 'words', g.island.pillW, true, frontW) : undefined;
  useLayoutEffect(() => {
    if (status !== 'recording') return;
    barRefs.current.forEach((el, i) => {
      if (el) el.style.transform = `scaleY(${SYNTHETIC_LEVELS[i % SYNTHETIC_LEVELS.length]})`;
    });
  }, [status]);
  return <section data-variant={variant} style={{ marginBottom: 20 }}>
    {!meetingOnly && <h2 style={{ font: '12px/20px sans-serif', marginBottom: 6 }}>{variant}</h2>}
    <div data-native-frame style={{
      display: 'flex', alignItems: g.floating ? 'flex-end' : undefined,
      width: g.windowW, height: expanded ? g.expandedH : g.collapsedH, marginInline: 'auto',
    }}>
      <OverlayIsland geometry={g} active={!idle} expanded={expanded} tapMissed={visual.showTapMissedLabel} fitW={fitW}>
        <OverlayPill geometry={g} visual={visual} status={status} barRefs={barRefs} liveWords={liveWords}
          fitW={fitW} onFrontWidth={variant === 'words' ? setFrontW : undefined} />
        {meeting ? <OverlayMeetingSuggestion geometry={g} expanded suggestion={{
          token: '88888888-8888-4888-8888-888888888888', title: 'Synthetic calendar: Product review', startMs: 1, endMs: 2,
        }} busy={false} error={null} onAccept={noop} onDismiss={noop} />
          // Production always mounts the dropdown, collapsed or not.
          : <OverlayDropdown geometry={g} expanded={expanded} status={status} stillConnecting={controlsState === 'still-connecting'}
            showTapMissed={false} disabled={false} autoPaste fileOutputEnabled={false}
            onToggleDisabled={noop} onToggleAutoPaste={noop} onOpenSettings={noop} />}
      </OverlayIsland>
    </div>
  </section>;
}

/** Production components + Rust-asserted geometry, synthetic levels/text only.
 * These browser fixtures cannot establish native focus, dictation or cost. */
export function VoiceIslandFixture({ meetingOnly = false }: { meetingOnly?: boolean }) {
  const query = new URLSearchParams(location.search);
  const pill = query.get('placement') === 'pill';
  const external = query.get('display') === 'external';
  const fallback = query.get('display') === 'fallback';
  const controlsState = controlsStates.find((state) => state === query.get('controls-state')) ?? 'idle';
  const requested = query.get('variant');
  const rows = meetingOnly ? ['meeting' as const]
    : variants.filter(v => (!requested || v === requested) && (pill || !PILL_ONLY.includes(v)));
  return <main data-visual-ready="true" data-fixture-synthetic={meetingOnly ? 'meeting-suggestion' : 'voice-island'}
    style={{ minHeight: '100vh', padding: 24, background: 'linear-gradient(135deg, var(--murmur-surface-container-high), var(--murmur-background))', color: 'var(--murmur-on-surface)' }}>
    {rows.map(variant => <FixtureRow key={variant} variant={variant} pill={pill} external={external} fallback={fallback}
      controlsState={controlsState} meetingOnly={meetingOnly} />)}
    {/* The floating pill carries its words itself; only the notch has a card. */}
    {!requested && !meetingOnly && !pill && <section data-live-words style={{ width: 460, height: 104, maxWidth: '100%', margin: '6px auto 0' }}>
      <DictationPreviewCard above={pill} stableText="Synthetic stable context from an authoritative fixture only"
        text="synthetic provisional tail that wraps onto a second line when space runs out" />
    </section>}
  </main>;
}
