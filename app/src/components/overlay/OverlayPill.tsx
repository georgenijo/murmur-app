import type { OverlayGeometry } from '../../lib/overlayGeometry';
import type { DictationStatus } from '../../lib/types';
import { DotField } from './DotField';
import type { ScopeStage, ProcessingStyle } from './dotScope';
import type { OverlayDeliveryCue } from '../../lib/hooks/useOverlayRuntime';
import type { OverlayVisual } from './deriveVisual';

interface OverlayPillProps {
  geometry: OverlayGeometry;
  visual: OverlayVisual;
  status: DictationStatus;
  scopeStage?: ScopeStage;
  processingStyle?: ProcessingStyle;
  fixtureAt?: number;
  smartAutoSummary?: OverlaySmartAutoSummary | null;
  deliveryCue?: OverlayDeliveryCue | null;
  onRetryDelivery?: () => void;
  onPauseDeliveryTimer?: () => void;
  onResumeDeliveryTimer?: () => void;
}

export type OverlaySmartAutoSummary =
  | { kind: 'ready'; deviceName: string; reason: string }
  | { kind: 'probing'; deviceName: string; phase: string }
  | { kind: 'blocked'; retryAfterMs: number | null };

/**
 * Priority-resolved status and decorative canvas share the plate below the
 * notch. Refusals/failures/delivery replace the field; provisional text belongs
 * only to the optional click-through preview. The parent owns native sizing,
 * hover and the sibling quick controls.
 */
export function OverlayPill({
  geometry,
  visual,
  scopeStage,
  processingStyle,
  fixtureAt,
  smartAutoSummary = null,
  deliveryCue = null,
  onRetryDelivery,
  onPauseDeliveryTimer,
  onResumeDeliveryTimer,
}: OverlayPillProps) {
  const topH = geometry.island.plateH + 2 * geometry.island.pad;
  const stage = visual.indicator.kind === 'recording' ? 'recording'
    : visual.indicator.kind === 'processing' ? 'processing'
      : visual.indicator.kind === 'idle' && scopeStage === 'done' ? 'done' : 'idle';
  const wingW = geometry.wingW;
  const { indicator } = visual;

  return (
    <div className="overlay-pill" style={{ height: geometry.collapsedH, position: 'relative' }}>
      {/* The native overlay is anchor-owned; this surface deliberately is not a
          drag region because arbitrary movement is neither persisted nor part
          of the placement contract. */}
      <div className="overlay-status-row flex items-center"
        data-field-active={stage !== 'idle' || undefined}
        role={stage !== 'idle' ? 'status' : undefined}
        aria-label={stage === 'recording' ? 'Recording' : stage === 'processing' ? 'Processing' : stage === 'done' ? 'Done' : undefined}
        style={{ position: 'absolute', left: 0, right: 0, top: geometry.island.notchH, height: topH }}>
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
                  ? 'Selected microphone unavailable. Open settings to choose another.'
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
              aria-label={deliveryCue?.message ?? 'Text copied to clipboard. Paste manually or try again.'}
              className={`text-[9px] leading-none font-semibold tracking-[-0.04em] ${deliveryCue?.kind === 'failed' ? 'text-red-300' : deliveryCue?.kind === 'clipboard_only' || deliveryCue?.kind === 'busy' || deliveryCue?.kind === 'empty' ? 'text-amber-300' : deliveryCue?.kind === 'retrying' ? 'text-white/70' : 'text-emerald-300'}`}
            >
              {deliveryCue?.kind === 'auto_pasted'
                ? '✓'
                : deliveryCue?.kind === 'retrying'
                  ? '…'
                  : deliveryCue?.kind === 'failed'
                    ? '!'
                    : deliveryCue?.kind === 'clipboard_only'
                      ? '↻'
                    : deliveryCue?.kind === 'busy'
                      ? '…'
                      : deliveryCue?.kind === 'empty'
                        ? '—'
                        : '⌘V'}
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
            <span
              role={smartAutoSummary ? 'status' : undefined}
              aria-live={smartAutoSummary ? 'polite' : undefined}
              aria-label={smartAutoSummary?.kind === 'ready'
                ? `Smart Auto next capture ready with ${smartAutoSummary.deviceName}. ${smartAutoSummary.reason}.`
                : smartAutoSummary?.kind === 'probing'
                  ? `Smart Auto background check for ${smartAutoSummary.deviceName}: ${smartAutoSummary.phase}. Audio is not transcribed or saved.`
                : smartAutoSummary?.kind === 'blocked'
                  ? smartAutoSummary.retryAfterMs === null
                    ? 'Smart Auto next capture blocked. Open settings to retry, verify signal, or pin an input.'
                    : `Smart Auto next capture blocked. Cooldown: about ${Math.max(1, Math.ceil(smartAutoSummary.retryAfterMs / 1000))} seconds remaining.`
                  : undefined}
              title={smartAutoSummary?.kind === 'ready'
                ? `Smart Auto: ${smartAutoSummary.deviceName}. ${smartAutoSummary.reason}.`
                : smartAutoSummary?.kind === 'probing'
                  ? `Smart Auto checking ${smartAutoSummary.deviceName}: ${smartAutoSummary.phase}`
                : smartAutoSummary?.kind === 'blocked'
                  ? smartAutoSummary.retryAfterMs === null
                    ? 'Smart Auto blocked. Open settings to retry, verify signal, or pin an input.'
                    : 'Smart Auto blocked. Waiting for cooldown.'
                  : undefined}
              className={smartAutoSummary?.kind === 'ready'
                ? 'text-emerald-300'
                : smartAutoSummary?.kind === 'probing'
                  ? 'text-sky-300 animate-pulse'
                : smartAutoSummary?.kind === 'blocked'
                  ? 'text-amber-300'
                  : 'text-white/40'}
              style={{ opacity: indicator.dimmed ? 0.15 : 1 }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                <rect x="9" y="1" width="6" height="12" rx="3" />
                <path d="M5 10a7 7 0 0 0 14 0" />
                <line x1="12" y1="17" x2="12" y2="21" />
              </svg>
            </span>
          )}
        </div>

        {/* Notch-obscured center — flex absorbs the physical notch width. */}
        <div className="flex-1" aria-hidden="true" />

        {/* Delivery remains actionable without taking focus from the editor. */}
        {indicator.kind === 'clipboardOnly' ? (
          <div className="shrink-0 flex items-center justify-center" style={{ width: 52 }}>
            {deliveryCue?.kind === 'auto_pasted' ? (
              <span className="text-[8px] font-medium leading-none text-emerald-200">Pasted</span>
            ) : deliveryCue?.kind === 'retrying' ? (
              <span className="text-[8px] font-medium leading-none text-white/70">Trying…</span>
            ) : deliveryCue?.kind === 'empty' ? (
              <span className="text-[8px] font-medium leading-none text-amber-200">Nothing</span>
            ) : (
              <button
                type="button"
                tabIndex={-1}
                title={deliveryCue?.message}
                aria-label="Try delivery again"
                className="whitespace-nowrap text-emerald-200 hover:text-white"
                style={{
                  fontSize: '8px',
                  fontWeight: 600,
                  lineHeight: 1,
                  letterSpacing: '-0.03em',
                }}
                onPointerDown={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  onPauseDeliveryTimer?.();
                }}
                onPointerLeave={onResumeDeliveryTimer}
                onPointerCancel={onResumeDeliveryTimer}
                onMouseDown={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                }}
                onDoubleClick={(event) => event.stopPropagation()}
                onClick={(event) => {
                  event.stopPropagation();
                  onRetryDelivery?.();
                }}
              >
                Try again
              </button>
            )}
          </div>
        ) : <div style={{ width: wingW }} aria-hidden="true" />}
      </div>
      <div className="overlay-field-slot" style={{
        top: geometry.island.notchH + geometry.island.pad,
        height: geometry.island.plateH,
        borderRadius: geometry.island.innerRadius,
        visibility: stage === 'idle' ? 'hidden' : undefined,
      }}>
        <DotField stage={stage} processingStyle={processingStyle} fixtureAt={fixtureAt} geometry={geometry.island} />
      </div>
    </div>
  );
}
