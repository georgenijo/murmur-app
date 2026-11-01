import { useLayoutEffect, useRef } from 'react';
import { OverlayIsland } from '../components/overlay/OverlayIsland';
import { OverlayPill } from '../components/overlay/OverlayPill';
import { OverlayDropdown } from '../components/overlay/OverlayDropdown';
import { OverlayMeetingSuggestion } from '../components/overlay/OverlayMeetingSuggestion';
import { DictationPreviewCard } from '../components/dictation-preview/DictationPreviewApp';
import { deriveVisual } from '../components/overlay/deriveVisual';
import geometry from '../components/overlay/overlay-geometry.fixture.json';
import type { OverlayGeometry } from '../lib/overlayGeometry';
import type { DictationStatus } from '../lib/types';

const noop = () => {};
const variants = ['idle', 'recording', 'processing', 'cancelled', 'secure', 'miss', 'microphone', 'delivery', 'expanded', 'meeting'] as const;
type Variant = typeof variants[number];
/** Fixed synthetic levels so recording screenshots are deterministic. */
const SYNTHETIC_LEVELS = [0.3, 0.55, 0.8, 1, 0.7, 0.45, 0.6, 0.9, 0.5, 0.35, 0.75, 0.95, 0.65, 0.4, 0.85, 0.55, 0.3, 0.2];

function geometryKey(variant: Variant, pill: boolean, external: boolean): keyof typeof geometry {
  if (variant === 'meeting') return pill ? 'pillMeetingSuggestion' : 'expandedMeetingNotched';
  if (variant === 'expanded') return pill ? 'pillExpanded' : 'expandedNotched';
  const activity = variant === 'idle' ? 'idle' : variant === 'recording' ? 'recording' : 'processing';
  if (pill) return activity === 'idle' ? 'pillIdle' : activity === 'recording' ? 'pillRecording' : 'pillProcessing';
  return `${activity}${external ? 'External' : 'Notched'}`;
}

function FixtureRow({ variant, pill, external, meetingOnly }: {
  variant: Variant; pill: boolean; external: boolean; meetingOnly: boolean;
}) {
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);
  const meeting = variant === 'meeting';
  const expanded = meeting || variant === 'expanded';
  const idle = variant === 'idle';
  const recording = variant === 'recording';
  const g: OverlayGeometry = geometry[geometryKey(variant, pill, external)];
  const status: DictationStatus = recording || ['cancelled', 'secure', 'miss', 'microphone'].includes(variant)
    ? 'recording' : variant === 'processing' ? 'processing' : 'idle';
  const visual = deriveVisual(status, variant === 'cancelled', variant === 'miss', false, false,
    variant === 'secure', false, variant === 'microphone' ? 'chooseMicrophone' : null,
    false, false, 'idle', variant === 'delivery');
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
      <OverlayIsland geometry={g} active={!idle} expanded={expanded} tapMissed={visual.showTapMissedLabel}>
        <OverlayPill geometry={g} visual={visual} status={status} barRefs={barRefs} />
        {meeting ? <OverlayMeetingSuggestion geometry={g} expanded suggestion={{
          token: '88888888-8888-4888-8888-888888888888', title: 'Synthetic calendar: Product review', startMs: 1, endMs: 2,
        }} busy={false} error={null} onAccept={noop} onDismiss={noop} />
          // Production always mounts the dropdown, collapsed or not.
          : <OverlayDropdown geometry={g} expanded={expanded} status="idle" stillConnecting={false}
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
  const requested = query.get('variant');
  const rows = meetingOnly ? ['meeting' as const] : variants.filter(v => !requested || v === requested);
  return <main data-visual-ready="true" data-fixture-synthetic={meetingOnly ? 'meeting-suggestion' : 'voice-island'}
    style={{ minHeight: '100vh', padding: 24, background: 'linear-gradient(135deg, var(--murmur-surface-container-high), var(--murmur-background))', color: 'var(--murmur-on-surface)' }}>
    {rows.map(variant => <FixtureRow key={variant} variant={variant} pill={pill} external={external} meetingOnly={meetingOnly} />)}
    {!requested && !meetingOnly && <section data-live-words style={{ width: 460, height: 104, maxWidth: '100%', margin: '6px auto 0' }}>
      <DictationPreviewCard above={pill} stableText="Synthetic stable context from an authoritative fixture only"
        text="synthetic provisional tail that wraps onto a second line when space runs out" />
    </section>}
  </main>;
}
