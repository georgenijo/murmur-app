import type { OverlayGeometry } from '../../lib/overlayGeometry';
import type { DictationStatus } from '../../lib/types';
import { BAR_COUNT } from '../../lib/hooks/useWaveform';
import type { OverlayVisual } from './deriveVisual';

interface OverlayPillProps {
  geometry: OverlayGeometry;
  visual: OverlayVisual;
  status: DictationStatus;
  partialText?: string;
  barRefs: React.MutableRefObject<(HTMLDivElement | null)[]>;
}

/**
 * Top-bar content: status indicator (left wing) + waveform (right wing). Purely
 * presentational — driven by the `visual` descriptor from `deriveVisual`.
 *
 * Each wing is a fixed `geometry.wingW` slot with content *centered* inside it
 * (not flush to the notch edge). The flex-1 spacer is the notch-obscured
 * center. Wider content (recording timer, "Tap missed" label) lives in the
 * dropdown row, below notch height. Does not own the island container
 * (sizing/hover/islandRef stay in OverlayWidget.tsx, since they also govern
 * the sibling dropdown).
 */
export function OverlayPill({
  geometry,
  visual,
  status,
  partialText = '',
  barRefs,
}: OverlayPillProps) {
  const topH = geometry.collapsedH;
  const wingW = geometry.wingW;
  const { indicator } = visual;

  return (
    <>
      {/* Top bar — the only draggable surface (keeps the dropdown buttons clickable).
          No horizontal padding: each wing owns its full wingW and centers content. */}
      <div data-tauri-drag-region className="flex items-center" style={{ height: topH }}>
        {/* Left wing — fixed wing slot, content centered */}
        <div
          className="shrink-0 flex items-center justify-center"
          style={{ width: wingW }}
        >
          {indicator.kind === 'calibrating' ? (
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="#92dbfe" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-label="calibrating overlay">
              <path d="M12 3v18M8 7l4-4 4 4M8 17l4 4 4-4" />
            </svg>
          ) : indicator.kind === 'cancelled' ? (
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#ef4444" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
              <line x1="6" y1="6" x2="18" y2="18" />
              <line x1="18" y1="6" x2="6" y2="18" />
            </svg>
          ) : indicator.kind === 'hotkeyMiss' ? (
            <span className="w-3 h-3 rounded-full border border-amber-400 text-amber-300 text-[8px] leading-none flex items-center justify-center font-bold">
              !
            </span>
          ) : indicator.kind === 'microphoneFailure' ? (
            indicator.failure === 'chooseMicrophone'
              || indicator.failure === 'openMicrophoneSettings' ? (
              <span
                role="status"
                aria-live="assertive"
                aria-label={indicator.failure === 'chooseMicrophone'
                  ? 'Selected microphone unavailable. Open Settings to choose another.'
                  : 'Microphone access denied. Open System Settings to grant access.'}
                className="flex h-4 w-4 items-center justify-center text-red-300"
              >
                <svg
                  aria-hidden="true"
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M9 9V5a3 3 0 0 1 5.6-1.5" />
                  <path d="M15 11.5V13a3 3 0 0 1-.4 1.5" />
                  <path d="M5 10v2a7 7 0 0 0 11.2 5.6" />
                  <path d="M19 10v2a7 7 0 0 1-.5 2.6" />
                  <path d="M12 19v3" />
                  <path d="M8 22h8" />
                  <path d="M3 3l18 18" />
                </svg>
              </span>
            ) : (
              <span
                role="status"
                aria-live="assertive"
                aria-label={indicator.failure === 'waitForPartialTranscription'
                  ? 'Microphone capture was interrupted. Waiting for the partial transcription.'
                  : 'Microphone capture failed. Try recording again.'}
                className="w-3 h-3 rounded-full border border-red-400 text-red-300 text-[8px] leading-none flex items-center justify-center font-bold"
              >
                !
              </span>
            )
          ) : indicator.kind === 'clipboardOnly' ? (
            <span
              role="status"
              aria-live="polite"
              aria-label="Text copied to clipboard. Paste manually."
              className="text-emerald-300 text-[9px] leading-none font-semibold tracking-[-0.04em]"
            >
              ⌘V
            </span>
          ) : indicator.kind === 'starting' ? (
            <span
              className={`w-2.5 h-2.5 rounded-full block ${indicator.slow ? 'bg-amber-400' : 'bg-sky-400'}`}
              style={{ animation: 'pulse 1s ease-in-out infinite' }}
              aria-label={indicator.slow ? 'still connecting microphone' : 'connecting microphone'}
            />
          ) : indicator.kind === 'recording' ? (
            <div className="w-2.5 h-2.5 rounded-full bg-red-500" style={{ animation: 'pulse 0.8s ease-in-out infinite' }} />
          ) : indicator.kind === 'meeting' ? (
            indicator.processing ? (
              <span className="block h-3 w-3 animate-spin rounded-full border-[1.5px] border-cyan-300/25 border-t-cyan-300" aria-label="finishing meeting transcript" />
            ) : (
              <span className="flex items-center gap-0.5" aria-label="meeting capture active">
                <span className="h-2 w-2 animate-pulse rounded-full bg-cyan-300" />
                <span className="h-2 w-2 animate-pulse rounded-full bg-violet-300" />
              </span>
            )
          ) : indicator.kind === 'processing' ? (
            <span className="w-3 h-3 border-[1.5px] border-white/20 border-t-white/70 rounded-full animate-spin block" />
          ) : indicator.kind === 'recovering' ? (
            <span className="w-3 h-3 border-[1.5px] border-amber-400/30 border-t-amber-300 rounded-full animate-spin block" aria-label="recovering microphone" />
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

        {/* Notch-obscured center — flex absorbs the physical notch width. */}
        <div className="flex-1" aria-hidden="true" />

        {/* Right wing — same fixed wing slot; waveform centered in the middle
            of the wing (not flush against the notch edge). */}
        <div
          className="shrink-0 flex items-center justify-center transition-opacity duration-300"
          style={{ width: wingW, opacity: visual.waveformVisible ? 1 : 0 }}
          aria-hidden={!visual.waveformVisible}
        >
          {status === 'recording' && partialText ? (
            <span
              role="status"
              aria-live="polite"
              aria-label={`Live transcription preview: ${partialText}`}
              className="block w-full truncate px-1 text-center text-[9px] leading-none text-white/80"
            >
              {partialText}
            </span>
          ) : <div className="flex items-center gap-[1.5px] h-4">
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
          </div>}
        </div>
      </div>
    </>
  );
}
