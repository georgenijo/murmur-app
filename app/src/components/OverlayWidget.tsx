import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../lib/log';
import { useOverlayRecordingStatus } from '../lib/hooks/useOverlayRecordingStatus';
import { useOverlayGeometry } from '../lib/hooks/useOverlayGeometry';
import { useOverlayExpansion } from '../lib/hooks/useOverlayExpansion';
import { useOverlayDone } from '../lib/hooks/useOverlayDone';
import { useOverlayRuntime } from '../lib/hooks/useOverlayRuntime';
import { useOverlaySettingsMirror } from '../lib/hooks/useOverlaySettingsMirror';
import { useRecordingControls } from '../lib/hooks/useRecordingControls';
import { OverlayIsland } from './overlay/OverlayIsland';
import { useModeRuntime } from '../lib/hooks/useModeRuntime';
import { useAudioInputInventory } from '../lib/hooks/useAudioInputInventory';
import { useSmartAutoMicrophoneStatus } from '../lib/hooks/useSmartAutoMicrophoneStatus';
import { deriveVisual } from './overlay/deriveVisual';
import { OverlayPill } from './overlay/OverlayPill';
import { OverlayDropdown } from './overlay/OverlayDropdown';
import { IDLE_MEETING_STATUS, type MeetingRuntimePhase, type MeetingRuntimeStatus } from '../lib/meetings';
import { smartAutoMicrophoneReasonLabel, smartAutoProbePhaseLabel } from '../lib/smartAutoMicrophone';
import { useMeetingSuggestion } from '../lib/hooks/useMeetingSuggestion';
import { OverlayMeetingSuggestion } from './overlay/OverlayMeetingSuggestion';
import type { OverlayContent } from '../lib/overlayGeometry';
import { useApplySavedOverlayOffset } from '../lib/hooks/useApplySavedOverlayOffset';
import { useWaveform } from '../lib/hooks/useWaveform';

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
  useApplySavedOverlayOffset(geometry, settingsMirror.overlayVerticalOffset);
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
  const showDone = useOverlayDone(status);
  const cueActive = calibrating || transforming || meetingBusy || runtime.deliveryCue !== null
    || runtime.showCancelled || runtime.showHotkeyMiss || runtime.showSecureField
    || runtime.showTransformBusy || runtime.showMicrophoneFailure !== null;
  const activity = status === 'recording' ? 'recording'
    : status !== 'idle' || showDone || cueActive ? 'processing' : 'idle';
  // The idle floating pill is invisible and click-through, so hover must not
  // open a card over whatever sits above the Dock.
  const pillHidden = settingsMirror.overlayPlacement === 'floating_pill' && activity === 'idle';
  const { phase, expanded, expandedRef, islandRef, onHoverStart, onHoverEnd } =
    useOverlayExpansion({ forcedOpen: showMeetingSuggestion, content: overlayContent,
      activity, placement: settingsMirror.overlayPlacement, hoverEnabled: !pillHidden });
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

  return (
    <div
      className={`relative flex h-full w-full ${geometry.floating ? 'items-end' : ''}`}
      style={{ background: 'transparent' }}
    >
      {/* Notch: the top bar matches the notch height and hover expands it
          downward. Floating pill: the capsule sits at the window bottom and
          hover grows it upward. The dropdown row is identical in both. */}
      <OverlayIsland
        ref={islandRef}
        geometry={geometry}
        expanded={expanded}
        active={activity !== 'idle'}
        tapMissed={visual.showTapMissedLabel}
        onMouseDown={calibrating || meetingBusy || showMeetingSuggestion ? undefined : recordingControls.handleMouseDown}
        onDoubleClick={calibrating || meetingBusy || showMeetingSuggestion ? undefined : recordingControls.handleDoubleClick}
        onClick={calibrating || meetingBusy || showMeetingSuggestion ? undefined : recordingControls.handleClick}
        onMouseEnter={calibrating ? undefined : onHoverStart}
        onMouseMove={calibrating ? undefined : onHoverStart}
        onMouseLeave={calibrating ? undefined : onHoverEnd}
      >
        <OverlayPill
          geometry={geometry}
          visual={visual}
          status={status}
          barRefs={waveform.barRefs}
          smartAutoSummary={smartAutoSummary}
          deliveryCue={runtime.deliveryCue}
          onRetryDelivery={runtime.retryDelivery}
          onPauseDeliveryTimer={runtime.pauseDeliveryTimer}
          onResumeDeliveryTimer={runtime.resumeDeliveryTimer}
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
            smartDestinationStatus={runtime.smartDestinationStatus}
            smartDestinationAppName={runtime.smartDestinationAppName}
            disabled={runtime.disabled}
            autoPaste={settingsMirror.autoPaste}
            fileOutputEnabled={settingsMirror.fileOutputEnabled}
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
      </OverlayIsland>
    </div>
  );
}
