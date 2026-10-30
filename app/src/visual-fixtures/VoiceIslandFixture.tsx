import { OverlayIsland } from '../components/overlay/OverlayIsland';
import { OverlayPill } from '../components/overlay/OverlayPill';
import { OverlayDropdown } from '../components/overlay/OverlayDropdown';
import { OverlayMeetingSuggestion } from '../components/overlay/OverlayMeetingSuggestion';
import { DictationPreviewCard } from '../components/dictation-preview/DictationPreviewApp';
import { deriveVisual } from '../components/overlay/deriveVisual';
import geometry from '../components/overlay/overlay-geometry.fixture.json';
import type { OverlayGeometry } from '../lib/overlayGeometry';
import type { DictationStatus } from '../lib/types';
import type { ProcessingStyle } from '../components/overlay/dotScope';

const noop = () => {};
const variants = ['idle', 'recording', 'sweep', 'flow', 'pulse', 'done', 'cancelled', 'secure', 'miss', 'microphone', 'delivery', 'expanded', 'meeting'] as const;

/** Production components + Rust-asserted geometry, synthetic levels/text only.
 * These browser fixtures cannot establish native focus, dictation or cost. */
export function VoiceIslandFixture({ meetingOnly = false }: { meetingOnly?: boolean }) {
  const query = new URLSearchParams(location.search);
  const below = query.get('placement') === 'below';
  const external = query.get('display') === 'external';
  const requested = query.get('variant');
  const rows = meetingOnly ? ['meeting' as const] : variants.filter(v => !requested || v === requested);
  return <main data-visual-ready="true" data-fixture-synthetic={meetingOnly ? 'meeting-suggestion' : 'voice-island'}
    style={{ minHeight: '100vh', padding: 24, background: 'linear-gradient(135deg, var(--murmur-surface-container-high), var(--murmur-background))', color: 'var(--murmur-on-surface)' }}>
    {rows.map(variant => {
      const meeting = variant === 'meeting';
      const expanded = meeting || variant === 'expanded';
      const idle = variant === 'idle';
      const recording = variant === 'recording';
      const key = meeting ? below ? 'belowMeetingSuggestion' : 'expandedMeetingNotched'
        : expanded ? below ? 'belowExpanded' : 'expandedNotched'
          : below ? idle ? 'belowIdle' : recording ? 'belowRecording' : 'belowProcessing'
            : idle ? external ? 'idleExternal' : 'idleNotched'
              : recording ? external ? 'recordingExternal' : 'recordingNotched'
                : external ? 'processingExternal' : 'processingNotched';
      const g: OverlayGeometry = geometry[key];
      const status: DictationStatus = recording || ['cancelled', 'secure', 'miss', 'microphone'].includes(variant)
        ? 'recording' : ['sweep', 'flow', 'pulse'].includes(variant) ? 'processing' : 'idle';
      const visual = deriveVisual(status, variant === 'cancelled', variant === 'miss', false, false,
        variant === 'secure', false, variant === 'microphone' ? 'chooseMicrophone' : null,
        false, false, 'idle', variant === 'delivery');
      return <section key={variant} data-variant={variant} style={{ marginBottom: 20 }}>
        {!meetingOnly && <h2 style={{ font: '12px/20px sans-serif', marginBottom: 6 }}>{variant}</h2>}
        <div data-native-frame style={{ width: g.windowW, height: expanded ? g.expandedH : g.collapsedH, marginInline: 'auto' }}>
          <OverlayIsland geometry={g} active={!idle} expanded={expanded} tapMissed={visual.showTapMissedLabel}>
            <OverlayPill geometry={g} visual={visual} status={status} fixtureAt={1.2}
              scopeStage={variant === 'done' ? 'done' : undefined}
              processingStyle={(['sweep', 'flow', 'pulse'].includes(variant) ? variant : 'flow') as ProcessingStyle} />
            {meeting ? <OverlayMeetingSuggestion geometry={g} expanded suggestion={{
              token: '88888888-8888-4888-8888-888888888888', title: 'Synthetic calendar: Product review', startMs: 1, endMs: 2,
            }} busy={false} error={null} onAccept={noop} onDismiss={noop} />
              : expanded && <OverlayDropdown geometry={g} expanded status="idle" stillConnecting={false}
                showTapMissed={false} disabled={false} autoPaste fileOutputEnabled={false}
                onToggleDisabled={noop} onToggleAutoPaste={noop} onOpenSettings={noop} />}
          </OverlayIsland>
        </div>
      </section>;
    })}
    {!requested && !meetingOnly && <section data-live-words style={{ width: 560, maxWidth: '100%', margin: '6px auto 0' }}>
      <DictationPreviewCard stableText="Synthetic stable context from an authoritative fixture only, with older words clipped at the head"
        text="Synthetic provisional tail that is clipped at the end when space runs out" />
    </section>}
  </main>;
}
