import type { OverlayGeometry } from '../../lib/overlayGeometry';
import type { DictationStatus } from '../../lib/types';
import { NOTCH_BAR_COUNT, PILL_BAR_COUNT } from '../../lib/hooks/useWaveform';
import type { OverlayDeliveryCue } from '../../lib/hooks/useOverlayRuntime';
import type { OverlayVisual } from './deriveVisual';

interface OverlayPillProps {
  geometry: OverlayGeometry;
  visual: OverlayVisual;
  status: DictationStatus;
  barRefs: React.MutableRefObject<(HTMLDivElement | null)[]>;
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

type IndicatorProps = Pick<OverlayPillProps, 'visual' | 'smartAutoSummary' | 'deliveryCue'>;
type RetryProps = Pick<OverlayPillProps, 'deliveryCue' | 'onRetryDelivery' | 'onPauseDeliveryTimer' | 'onResumeDeliveryTimer'>;

/**
 * Top-bar content, purely presentational and driven by `deriveVisual`.
 *
 * Notch: the status indicator sits in the left wing and the waveform in the
 * right wing; the flex spacer is the notch-obscured centre. Floating pill: the
 * indicator and a wider waveform share one centred row inside the capsule.
 * Delivery cues keep their inline retry in both layouts. The parent owns the
 * island container, native sizing, and hover.
 */
export function OverlayPill({
  geometry,
  visual,
  status,
  barRefs,
  smartAutoSummary = null,
  deliveryCue = null,
  onRetryDelivery,
  onPauseDeliveryTimer,
  onResumeDeliveryTimer,
}: OverlayPillProps) {
  const topH = geometry.collapsedH;
  const wingW = geometry.wingW;
  const delivery = visual.indicator.kind === 'clipboardOnly';
  const indicator = <StatusIndicator visual={visual} smartAutoSummary={smartAutoSummary} deliveryCue={deliveryCue} />;
  const retryProps = { deliveryCue, onRetryDelivery, onPauseDeliveryTimer, onResumeDeliveryTimer };

  if (geometry.floating) {
    return (
      <div className="overlay-pill-row flex items-center justify-center gap-2.5 px-3" style={{ height: topH }}>
        <div className="shrink-0 flex items-center justify-center">{indicator}</div>
        {delivery ? <DeliveryRetry {...retryProps} /> : (
          <Waveform barRefs={barRefs} count={PILL_BAR_COUNT} status={status} visible={visual.waveformVisible} height={16} />
        )}
      </div>
    );
  }

  return (
    // The native overlay is anchor-owned; this surface deliberately is not a
    // drag region because arbitrary movement is neither persisted nor part of
    // the placement contract.
    <div className="flex items-center" style={{ height: topH }}>
      <div className="shrink-0 flex items-center justify-center" style={{ width: wingW }}>
        {indicator}
      </div>
      {/* Notch-obscured center — flex absorbs the physical notch width. */}
      <div className="flex-1" aria-hidden="true" />
      <div className="shrink-0 flex items-center justify-center" style={{ width: wingW }}>
        {delivery ? <DeliveryRetry {...retryProps} compact /> : (
          <Waveform barRefs={barRefs} count={NOTCH_BAR_COUNT} status={status} visible={visual.waveformVisible} height={14} />
        )}
      </div>
    </div>
  );
}

function Waveform({ barRefs, count, status, visible, height }: {
  barRefs: OverlayPillProps['barRefs'];
  count: number;
  status: DictationStatus;
  visible: boolean;
  height: number;
}) {
  return (
    <div
      className="overlay-waveform flex items-center gap-[1.5px]"
      data-recording={status === 'recording' || undefined}
      style={{ height, opacity: visible ? 1 : 0 }}
      aria-hidden="true"
    >
      {Array.from({ length: count }, (_, i) => (
        <div
          key={i}
          ref={(el) => { barRefs.current[i] = el; }}
          className="overlay-waveform-bar w-[2px] rounded-full bg-white/90"
          style={{ height }}
        />
      ))}
    </div>
  );
}

function StatusIndicator({ visual, smartAutoSummary = null, deliveryCue = null }: IndicatorProps) {
  const { indicator } = visual;
  return (
    <>
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
    </>
  );
}

/** `compact` fits the 28pt notch wing: the retry becomes an icon there. */
function DeliveryRetry({ deliveryCue = null, onRetryDelivery, onPauseDeliveryTimer, onResumeDeliveryTimer, compact = false }: RetryProps & { compact?: boolean }) {
  return (
    <>
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
          {compact ? (
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
              <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
              <path d="M21 3v5h-5" />
            </svg>
          ) : 'Try again'}
        </button>
      )}
    </>
  );
}
