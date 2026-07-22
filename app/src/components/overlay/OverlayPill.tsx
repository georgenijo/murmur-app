import type { OverlayGeometry } from '../../lib/overlayGeometry';
import type { DictationStatus } from '../../lib/types';
import { BAR_COUNT } from '../../lib/hooks/useWaveform';
import type { OverlayVisual } from './deriveVisual';

interface OverlayPillProps {
  geometry: OverlayGeometry;
  visual: OverlayVisual;
  status: DictationStatus;
  barRefs: React.MutableRefObject<(HTMLDivElement | null)[]>;
}

/**
 * Top-bar content: status indicator (left wing) + waveform (right wing). Purely
 * presentational — driven by the `visual` descriptor from `deriveVisual`.
 *
 * The wings are the only strips clear of the physical notch, so they hold ONLY
 * these two narrow elements. Wider content (recording timer, "Tap missed" label)
 * lives in the dropdown row, below notch height. Does not own the island
 * container (sizing/hover/islandRef stay in OverlayWidget.tsx, since they also
 * govern the sibling dropdown).
 */
export function OverlayPill({
  geometry,
  visual,
  status,
  barRefs,
}: OverlayPillProps) {
  const topH = geometry.collapsedH;
  const { indicator } = visual;

  return (
    <>
      {/* Top bar — the only draggable surface (keeps the dropdown buttons clickable) */}
      <div data-tauri-drag-region className="flex items-center" style={{ height: topH, paddingLeft: 10, paddingRight: 10 }}>
        {/* Left wing — mic icon (idle) or red dot (recording) or spinner (processing) or red X (cancelled) or amber ! (hotkey miss), all same position */}
        <div className="shrink-0 w-3 h-3 flex items-center justify-center">
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
            <span className="w-3 h-3 border-[1.5px] border-white/20 border-t-white/70 rounded-full animate-spin block" />
          ) : indicator.kind === 'secureField' ? (
            // Brief flash when a secure/password field is refused (issue #312).
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-label="secure field">
              <rect x="5" y="11" width="14" height="9" rx="2" />
              <path d="M8 11V7a4 4 0 0 1 8 0v4" />
            </svg>
          ) : indicator.kind === 'transformBusy' ? (
            // Brief flash when a transform keypress was refused — something
            // else (dictation/benchmark/…) owns the pipeline (issue #329).
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-label="transform busy">
              <circle cx="12" cy="12" r="9" />
              <polyline points="12 7 12 12 15.5 14" />
            </svg>
          ) : indicator.kind === 'transforming' ? (
            // "Transforming…" — local LLM is thinking (issue #312).
            <span className="w-2.5 h-2.5 rounded-full bg-violet-400 block" style={{ animation: 'pulse 0.8s ease-in-out infinite' }} />
          ) : (
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="rgba(255,255,255,0.4)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ opacity: indicator.dimmed ? 0.15 : 1 }}>
              <rect x="9" y="1" width="6" height="12" rx="3" />
              <path d="M5 10a7 7 0 0 0 14 0" />
              <line x1="12" y1="17" x2="12" y2="21" />
            </svg>
          )}
        </div>

        {/* This spacer is intentionally the notch-obscured center region. */}
        <div className="flex-1" aria-hidden="true" />

        {/* Right wing — waveform (only when recording) */}
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
      </div>
    </>
  );
}
