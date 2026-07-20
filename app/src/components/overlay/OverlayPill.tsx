import { useEffect, useState } from 'react';
import type { OverlayGeometry } from '../../lib/overlayGeometry';
import type { DictationStatus } from '../../lib/types';
import { BAR_COUNT } from '../../lib/hooks/useWaveform';
import type { OverlayVisual } from './deriveVisual';
import type { OverlayPreviewPresentation } from './previewPresentation';

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${String(s).padStart(2, '0')}`;
}

interface OverlayPillProps {
  geometry: OverlayGeometry;
  visual: OverlayVisual;
  status: DictationStatus;
  previewPresentation: OverlayPreviewPresentation;
  previewRowVisible: boolean;
  barRefs: React.MutableRefObject<(HTMLDivElement | null)[]>;
}

/**
 * Top-bar content (status indicator + inline timer + waveform) plus the
 * below-notch preview row. Purely presentational — driven by the `visual`
 * descriptor from `deriveVisual` and the pre-computed preview presentation.
 * Does not own the island container (sizing/hover/islandRef stay in
 * OverlayWidget.tsx, since they also govern the sibling dropdown).
 */
export function OverlayPill({
  geometry,
  visual,
  status,
  previewPresentation,
  previewRowVisible,
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

  return (
    <>
      {/* Top bar — the only draggable surface (keeps the dropdown buttons clickable) */}
      <div data-tauri-drag-region className="flex items-center" style={{ height: topH, paddingLeft: 10, paddingRight: 10 }}>
        {/* Left side — mic icon (idle) or red dot (recording) or spinner (processing) or red X (cancelled), all same position */}
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
          ) : (
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="rgba(255,255,255,0.4)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ opacity: indicator.dimmed ? 0.15 : 1 }}>
              <rect x="9" y="1" width="6" height="12" rx="3" />
              <path d="M5 10a7 7 0 0 0 14 0" />
              <line x1="12" y1="17" x2="12" y2="21" />
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

      {/* The physical notch hides the top-bar center. Put preview/status below it. */}
      {previewRowVisible && (
        <div
          aria-label={previewPresentation.unavailable
            ? 'Live transcript preview unavailable'
            : 'Provisional transcript preview'}
          className="flex items-center gap-2 px-3 pointer-events-none"
          style={{ height: geometry.previewRowH }}
        >
          {previewPresentation.unavailable ? (
            <>
              <span className="shrink-0 text-[8px] uppercase tracking-[0.12em] text-white/45">
                Final only
              </span>
              <span className="min-w-0 truncate text-[10px] text-white/65">
                Live preview unavailable for Parakeet
              </span>
            </>
          ) : (
            <>
              <span className="shrink-0 text-[8px] uppercase tracking-[0.12em] text-amber-300/85">
                Provisional
              </span>
              <span className="min-w-0 truncate text-[10px] text-white/80">
                {previewPresentation.previewText}
              </span>
            </>
          )}
        </div>
      )}
    </>
  );
}
