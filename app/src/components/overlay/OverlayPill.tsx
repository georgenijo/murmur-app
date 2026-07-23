import { useEffect, useState } from 'react';
import type { OverlayGeometry } from '../../lib/overlayGeometry';
import type { DictationStatus } from '../../lib/types';
import { BAR_COUNT } from '../../lib/hooks/useWaveform';
import { OVERLAY_STATE_MS, OVERLAY_ACTIVE_EASE } from '../../lib/overlayMotion';
import type { OverlayVisual } from './deriveVisual';
import { ThinkingOrb } from 'thinking-orbs';

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${String(s).padStart(2, '0')}`;
}

interface OverlayPillProps {
  geometry: OverlayGeometry;
  visual: OverlayVisual;
  status: DictationStatus;
  barRefs: React.MutableRefObject<(HTMLDivElement | null)[]>;
}

/**
 * Top-bar content: status indicator + inline timer + waveform. Purely
 * presentational — driven by the `visual` descriptor from `deriveVisual`.
 * Does not own the island container (sizing/hover/islandRef stay in
 * OverlayWidget.tsx, since they also govern the sibling dropdown).
 */
export function OverlayPill({
  geometry,
  visual,
  status,
  barRefs,
}: OverlayPillProps) {
  const [elapsed, setElapsed] = useState(0);

  // Track recording elapsed time for the inline timer (recording + hover only).
  useEffect(() => {
    if (status !== 'recording') { setElapsed(0); return; }
    const start = Date.now();
    setElapsed(0);
    const id = setInterval(() => setElapsed(Math.floor((Date.now() - start) / 1000)), 250);
    return () => clearInterval(id);
  }, [status]);

  const topH = geometry.collapsedH;
  const { indicator } = visual;
  // The final indicator branch renders both idle and disabled from one node so
  // the enable↔disable toggle can animate; this flag drives the morph.
  const micDisabled = indicator.kind === 'disabled';

  return (
    <>
      {/* Top bar — the only draggable surface (keeps the dropdown buttons clickable) */}
      <div data-tauri-drag-region className="flex items-center" style={{ height: topH, paddingLeft: 10, paddingRight: 10 }}>
        {/* Left side — mic icon (idle) or red dot (recording) or thinking orb (processing) or red X (cancelled), all same position.
            The processing orb needs a larger box (20px) than the 12px icons; during processing there's no timer/waveform, so growing the slot doesn't crowd anything. */}
        <div className={`shrink-0 flex items-center justify-center ${indicator.kind === 'processing' ? 'w-5 h-5' : 'w-3 h-3'}`}>
          {indicator.kind === 'cancelled' ? (
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#ef4444" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
              <line x1="6" y1="6" x2="18" y2="18" />
              <line x1="18" y1="6" x2="6" y2="18" />
            </svg>
          ) : indicator.kind === 'hotkeyMiss' ? (
            <span className="w-3 h-3 rounded-full border border-amber-400 text-amber-300 text-[8px] leading-none flex items-center justify-center font-bold">
              !
            </span>
          ) : indicator.kind === 'recording' ? (
            <div className="w-2.5 h-2.5 rounded-full bg-red-500" style={{ animation: 'pulse 0.8s ease-in-out infinite' }} />
          ) : indicator.kind === 'processing' ? (
            // Thinking-orb (MIT, thinking-orbs) — "working" state signals transcription in progress.
            <ThinkingOrb state="working" size={20} theme="dark" aria-label="Transcribing" />
          ) : (
            /* Idle OR globally-disabled: ONE persistent mic node so the enable↔disable
               toggle morphs instead of hard-swapping two SVGs. The stroke crossfades
               white↔red and the slash draws itself in/out (pathLength-normalized
               dashoffset). Disabled is a distinct shape (slashed red mic), not a dimmed
               mic — at 12px an opacity change is unreadable, and this state silently
               swallows every recording. */
            <svg
              width="12" height="12" viewBox="0 0 24 24" fill="none"
              strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
              style={{
                stroke: micDisabled ? '#ef4444' : 'rgba(255,255,255,0.4)',
                transition: `stroke ${OVERLAY_STATE_MS}ms ${OVERLAY_ACTIVE_EASE}`,
              }}
              aria-label={micDisabled ? 'Murmur is disabled' : undefined}
              role={micDisabled ? 'img' : undefined}
            >
              <rect x="9" y="1" width="6" height="12" rx="3" />
              <path d="M5 10a7 7 0 0 0 14 0" />
              <line x1="12" y1="17" x2="12" y2="21" />
              {/* Slash: always mounted, red, drawn in only when disabled. pathLength=1
                  normalizes the diagonal so dashoffset 1→0 sweeps it on (and 0→1
                  retracts it on re-enable). */}
              <line
                x1="3" y1="3" x2="21" y2="21" stroke="#ef4444"
                pathLength={1} strokeDasharray={1}
                style={{
                  strokeDashoffset: micDisabled ? 0 : 1,
                  transition: `stroke-dashoffset ${OVERLAY_STATE_MS}ms ${OVERLAY_ACTIVE_EASE}`,
                }}
              />
            </svg>
          )}
        </div>

        {/* Recording time remains in the visible left wing, outside the physical notch. */}
        {status === 'recording' && (
          <span className="shrink-0 text-white/60 tabular-nums" style={{ marginLeft: 7, fontSize: 11 }}>
            {formatElapsed(elapsed)}
          </span>
        )}

        {/* This spacer is intentionally the notch-obscured center region. */}
        <div className="flex-1" aria-hidden="true" />

        {/* Right side — waveform (only when active) */}
        {visual.showTapMissedLabel ? (
          <span className="shrink-0 text-amber-300 text-[10px] font-medium">
            Tap missed
          </span>
        ) : (
          <div
            className="flex items-center gap-[1.5px] h-4 shrink-0 transition-opacity duration-300"
            style={{ opacity: visual.waveformVisible ? 1 : 0 }}
          >
            {Array.from({ length: BAR_COUNT }, (_, i) => (
              <div
                key={i}
                ref={el => { barRefs.current[i] = el; }}
                className="w-[2px] rounded-full bg-white/90"
                style={{
                  height: '2px',
                  transition: `height ${status === 'recording' ? '50ms' : '300ms'} ease-out`,
                }}
              />
            ))}
          </div>
        )}
      </div>
    </>
  );
}
