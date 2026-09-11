import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../lib/log';
import { useOverlayRecordingStatus } from '../lib/hooks/useOverlayRecordingStatus';
import { useOverlayGeometry } from '../lib/hooks/useOverlayGeometry';
import { useOverlayExpansion } from '../lib/hooks/useOverlayExpansion';
import { useOverlayRuntime } from '../lib/hooks/useOverlayRuntime';
import { useOverlaySettingsMirror } from '../lib/hooks/useOverlaySettingsMirror';
import { useRecordingControls } from '../lib/hooks/useRecordingControls';
import { useWaveform } from '../lib/hooks/useWaveform';
import { useModeRuntime } from '../lib/hooks/useModeRuntime';
import { useAudioInputInventory } from '../lib/hooks/useAudioInputInventory';
import { useSmartAutoMicrophoneStatus } from '../lib/hooks/useSmartAutoMicrophoneStatus';
import { OVERLAY_ISLAND_TRANSITION } from '../lib/overlayMotion';
import { deriveVisual } from './overlay/deriveVisual';
import { OverlayPill } from './overlay/OverlayPill';
import { OverlayDropdown } from './overlay/OverlayDropdown';
import { IDLE_MEETING_STATUS, type MeetingRuntimePhase, type MeetingRuntimeStatus } from '../lib/meetings';
import { smartAutoMicrophoneReasonLabel, smartAutoProbePhaseLabel } from '../lib/smartAutoMicrophone';
import { useMeetingSuggestion } from '../lib/hooks/useMeetingSuggestion';
import { OverlayMeetingSuggestion } from './overlay/OverlayMeetingSuggestion';
import type { OverlayContent } from '../lib/overlayGeometry';

export function OverlayWidget() {
  const [calibrating, setCalibrating] = useState(false);

  // Shared mutable state written synchronously by both useOverlayRuntime's
  // Tauri listeners and useOverlaySettingsMirror's applySettingsSnapshot.
  // Created here (rather than inside either hook) because each hook needs to
  // write into it and neither can be constructed from the other's return
  // value without an artificial call-order dependency — see the doc comments
  // on useOverlayRuntime / useOverlaySettingsMirror.
  const [disabled, setDisabled] = useState(false);
  const [showHotkeyMiss, setShowHotkeyMiss] = useState(false);
  const { status, statusRef } = useOverlayRecordingStatus();
  // True while the local-LLM transform is thinking (issue #312 PR-C2). Driven
  // by the broadcast `transform-state-changed` event; the overlay is a
  // separate webview so it listens directly.
  const [transforming, setTransforming] = useState(false);
  const [meetingPhase, setMeetingPhase] = useState<MeetingRuntimePhase>('idle');
  const [stillConnecting, setStillConnecting] = useState(false);
  const hotkeyMissFeedbackRef = useRef(false);
  const meetingSuggestion = useMeetingSuggestion();
  const meetingBusy = meetingPhase !== 'idle' && meetingPhase !== 'failed';
  const showMeetingSuggestion = meetingSuggestion.suggestion !== null
    && status === 'idle'
    && !meetingBusy
    && !transforming
    && !disabled
    && !calibrating;
  const overlayContent: OverlayContent = showMeetingSuggestion
    ? 'meeting_suggestion'
    : 'controls';
  const geometry = useOverlayGeometry(overlayContent);

  const settingsMirror = useOverlaySettingsMirror({ setDisabled, setShowHotkeyMiss, hotkeyMissFeedbackRef });
  const smartAutoStatus = useSmartAutoMicrophoneStatus(settingsMirror.smartAuto);
  const smartAutoInventory = useAudioInputInventory(settingsMirror.smartAuto !== null);
  const smartAutoReady = smartAutoStatus.view.kind === 'resolved'
    && smartAutoStatus.view.status.state === 'ready'
    ? smartAutoStatus.view.status
    : null;
  const smartAutoProbing = smartAutoStatus.view.kind === 'resolved'
    && smartAutoStatus.view.status.state === 'probing'
    ? smartAutoStatus.view.status
    : null;
  const smartAutoBlocked = smartAutoStatus.view.kind === 'resolved'
    && smartAutoStatus.view.status.state === 'blocked'
    ? smartAutoStatus.view.status
    : null;
  const smartAutoSummary = smartAutoReady
    ? {
      kind: 'ready' as const,
      deviceName: smartAutoInventory.inventory?.devices.find(
        (device) => device.id === smartAutoReady.deviceId,
      )?.name ?? 'verified microphone',
      reason: smartAutoMicrophoneReasonLabel(smartAutoReady.reason),
    }
    : smartAutoProbing
      ? {
        kind: 'probing' as const,
        deviceName: smartAutoInventory.inventory?.devices.find(
          (device) => device.id === smartAutoProbing.deviceId,
        )?.name ?? 'included microphone',
        phase: smartAutoProbePhaseLabel(smartAutoProbing.phase),
      }
      : smartAutoBlocked
        ? { kind: 'blocked' as const, retryAfterMs: smartAutoBlocked.retryAfterMs }
        : smartAutoStatus.view.kind === 'unavailable'
          ? { kind: 'blocked' as const, retryAfterMs: null }
          : null;

  const runtime = useOverlayRuntime({
    status, statusRef, disabled, setDisabled, showHotkeyMiss, setShowHotkeyMiss, hotkeyMissFeedbackRef,
  });

  // The expansion controller owns the entire expand/collapse + surface lifecycle:
  // dwell/collapse/shrink timers, the serialized set_overlay_expanded writer, and
  // the single cursor poller. It is the only writer to the native resize path.
  const { phase, expanded, expandedRef, islandRef, onHoverStart, onHoverEnd } =
    useOverlayExpansion({ forcedOpen: showMeetingSuggestion, content: overlayContent });

  const waveform = useWaveform(status);
  const modeRuntime = useModeRuntime();

  const recordingControls = useRecordingControls({
    status, statusRef, disabledRef: runtime.disabledRef, expandedRef,
  });

  const visual = deriveVisual(
    status,
    runtime.showCancelled,
    runtime.showHotkeyMiss,
    runtime.disabled,
    transforming,
    runtime.showSecureField,
    runtime.showTransformBusy,
    runtime.showMicrophoneFailure,
    stillConnecting,
    calibrating,
    meetingPhase,
    runtime.showClipboardOnly,
  );

  useEffect(() => {
    void invoke<MeetingRuntimeStatus>('get_meeting_status')
      .then((meeting) => setMeetingPhase(meeting.phase))
      .catch(() => setMeetingPhase(IDLE_MEETING_STATUS.phase));
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<MeetingRuntimeStatus>('meeting-status-changed', (event) => {
      setMeetingPhase(event.payload.phase);
    }).then((fn) => {
      if (cancelled) fn(); else unlisten = fn;
    }).catch(() => {
      // Non-Tauri previews do not expose the native event bridge.
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  useEffect(() => {
    if (!geometry) return;
    void invoke('set_overlay_vertical_offset', { offset: settingsMirror.overlayVerticalOffset }).catch((error) => {
      flog.warn('overlay', 'could not apply calibrated offset', { error: String(error) });
    });
  }, [geometry, settingsMirror.overlayVerticalOffset]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    listen<{ active?: unknown }>('overlay-calibration-changed', (event) => {
      setCalibrating(event.payload?.active === true);
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    }).catch(() => {
      // Without the native event bridge, calibration stays inactive.
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (status !== 'starting') setStillConnecting(false);
  }, [status]);

  useEffect(() => {
    if (status === 'idle' && !meetingBusy && !transforming && !disabled && !calibrating) return;
    meetingSuggestion.clear();
  }, [status, meetingBusy, transforming, disabled, calibrating, meetingSuggestion.clear]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen('audio-initialization-stalled', () => {
      setStillConnecting(true);
    }).then((fn) => {
      if (cancelled) fn(); else unlisten = fn;
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Track the transform flow's thinking phase for the overlay indicator.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<unknown>('transform-state-changed', (event) => {
      const payload = event.payload as { state?: unknown } | null;
      const state = payload && typeof payload === 'object' ? payload.state : undefined;
      setTransforming(state === 'thinking');
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Log mount/unmount.
  useEffect(() => {
    flog.info('overlay', 'mounted');
    return () => { flog.info('overlay', 'unmounted'); };
  }, []);

  // Refresh quick-control values from localStorage as the card starts opening,
  // so the dropdown (revealed once the resize acks) shows current settings. The
  // overlay has no shared React settings context, so it re-reads on each open.
  const { refresh: refreshSettingsMirror } = settingsMirror;
  useEffect(() => {
    if (phase !== 'opening') return;
    refreshSettingsMirror();
  }, [phase, refreshSettingsMirror]);

  // Restore saved position (Rust handles default positioning)
  // TODO: re-enable after notch positioning is stable.
  // Both save (onMoved) and restore are disabled to avoid saving programmatic repositions.

  // All hooks are above this line. The overlay window is transparent, so returning
  // null before geometry arrives (~1 IPC round-trip after mount) paints nothing
  // rather than TS fallback pixels — no mis-sized flash, no fallback constants.
  if (!geometry) return null;
  const topH = geometry.collapsedH;
  // Only the genuinely off/idle surface tucks the empty right wing beneath the
  // notch. Recording and processing keep the full top bar even without hover so
  // their right-side indicators never disappear.
  const compactIdle = status === 'idle'
    && !meetingBusy
    && !expanded
    && !calibrating
    && !showMeetingSuggestion;
  const pillW = compactIdle ? geometry.pillIdleW : geometry.pillActiveW;
  const pillMargin = compactIdle ? geometry.pillMarginIdle : geometry.pillMarginActive;

  return (
    <div
      className="relative flex h-full w-full"
      style={{ background: 'transparent' }}
      onMouseDown={calibrating || meetingBusy || showMeetingSuggestion ? undefined : recordingControls.handleMouseDown}
      onDoubleClick={calibrating || meetingBusy || showMeetingSuggestion ? undefined : recordingControls.handleDoubleClick}
      onClick={calibrating || meetingBusy || showMeetingSuggestion ? undefined : recordingControls.handleClick}
      onMouseEnter={calibrating ? undefined : onHoverStart}
      onMouseMove={calibrating ? undefined : onHoverStart}
    >
      {/* Dynamic Island: top bar matches notch height; hover expands it downward
          to reveal the quick-settings dropdown. Idle/recording only changes the
          top bar — the dropdown row is identical. */}
      <div
        ref={islandRef}
        className="overlay-island cursor-pointer select-none overflow-hidden"
        onMouseEnter={calibrating ? undefined : onHoverStart}
        onMouseMove={calibrating ? undefined : onHoverStart}
        onMouseLeave={calibrating ? undefined : onHoverEnd}
        style={{
          position: 'relative',
          borderRadius: '0 0 12px 12px',
          // Left anchored: idle tucks only the empty right wing beneath the
          // notch; hover grows that edge back to the full active rectangle.
          width: pillW,
          height: topH + (expanded ? geometry.dropdownH : 0),
          marginLeft: pillMargin,
          background: 'rgba(20, 20, 20, 0.92)',
          boxShadow: visual.showTapMissedLabel ? 'inset 0 -2px 0 rgba(245,158,11,0.9), 0 3px 16px rgba(245,158,11,0.22)' : 'none',
          backdropFilter: 'blur(40px)',
          WebkitBackdropFilter: 'blur(40px)',
          transition: OVERLAY_ISLAND_TRANSITION,
        }}
      >
        <OverlayPill
          geometry={geometry}
          visual={visual}
          status={status}
          barRefs={waveform.barRefs}
          smartAutoSummary={smartAutoSummary}
        />
        {showMeetingSuggestion && meetingSuggestion.suggestion ? (
          <OverlayMeetingSuggestion
            geometry={geometry}
            expanded={expanded}
            suggestion={meetingSuggestion.suggestion}
            busy={meetingSuggestion.busy}
            error={meetingSuggestion.error}
            onAccept={() => void meetingSuggestion.accept()}
            onDismiss={() => void meetingSuggestion.dismiss()}
          />
        ) : (
          <OverlayDropdown
            geometry={geometry}
            expanded={expanded}
            status={status}
            stillConnecting={stillConnecting}
            showTapMissed={visual.showTapMissedLabel}
            disabled={runtime.disabled}
            autoPaste={settingsMirror.autoPaste}
            fileOutputEnabled={settingsMirror.fileOutputEnabled}
            recordingShortcutHint={settingsMirror.recordingShortcutHint}
            mode={modeRuntime.status}
            onCycleMode={(event) => {
              event.stopPropagation();
              void modeRuntime.cycle();
            }}
            onToggleDisabled={settingsMirror.handleToggleDisabled}
            onToggleAutoPaste={settingsMirror.handleToggleAutoPaste}
            onOpenSettings={settingsMirror.handleOpenSettings}
          />
        )}
      </div>
    </div>
  );
}
