import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useOverlayRecordingStatus } from '../lib/hooks/useOverlayRecordingStatus';
import { useOverlayGeometry } from '../lib/hooks/useOverlayGeometry';
import { useOverlayExpansion } from '../lib/hooks/useOverlayExpansion';
import { useOverlayDone } from '../lib/hooks/useOverlayDone';
import { useOverlayRuntime } from '../lib/hooks/useOverlayRuntime';
import { useOverlaySettingsMirror } from '../lib/hooks/useOverlaySettingsMirror';
import { useRecordingControls } from '../lib/hooks/useRecordingControls';
import { OverlayIsland } from './overlay/OverlayIsland';
import { useAudioInputInventory } from '../lib/hooks/useAudioInputInventory';
import { useSmartAutoMicrophoneStatus } from '../lib/hooks/useSmartAutoMicrophoneStatus';
import { deriveVisual } from './overlay/deriveVisual';
import { OverlayPill } from './overlay/OverlayPill';
import { OverlayDropdown } from './overlay/OverlayDropdown';
import { OverlayTranscript } from './overlay/OverlayTranscript';
import { IDLE_MEETING_STATUS, type MeetingRuntimePhase, type MeetingRuntimeStatus } from '../lib/meetings';
import { smartAutoMicrophoneReasonLabel, smartAutoProbePhaseLabel } from '../lib/smartAutoMicrophone';
import { useMeetingSuggestion } from '../lib/hooks/useMeetingSuggestion';
import { OverlayMeetingSuggestion } from './overlay/OverlayMeetingSuggestion';
import { overlayPillSize } from '../lib/overlayGeometry';
import { overlayPillFitWidth } from '../lib/overlayPillFit';
import type { OverlayContent } from '../lib/overlayGeometry';
import { useDictationPartial } from '../lib/hooks/useDictationPartial';
import { useApplySavedOverlayOffset } from '../lib/hooks/useApplySavedOverlayOffset';
import { useWaveform } from '../lib/hooks/useWaveform';
import { useLiveWordsLift } from '../lib/hooks/useLiveWordsLift';
import { useOverlayMaterial } from '../lib/hooks/useOverlayMaterial';
import { MODEL_SETTINGS_TARGET, modelUnavailableWord } from '../lib/modelUnavailable';

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
  const [meetingPhase, setMeetingPhase] = useState<MeetingRuntimePhase>('idle');
  const [stillConnecting, setStillConnecting] = useState(false);
  const hotkeyMissFeedbackRef = useRef(false);
  const meetingSuggestion = useMeetingSuggestion();
  const meetingBusy = meetingPhase !== 'idle' && meetingPhase !== 'failed';
  const showMeetingSuggestion = meetingSuggestion.suggestion !== null
    && status === 'idle'
    && !meetingBusy
    && !disabled
    && !calibrating;
  // The geometry profile: the transcript card needs none of its own, its
  // size arrives with the expand acknowledgment (`overlay-surface-changed`),
  // and a profile change would blank the pill mid-dictation.
  const geometry = useOverlayGeometry(showMeetingSuggestion ? 'meeting_suggestion' : 'controls');

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
  const cueActive = calibrating || meetingBusy || runtime.deliveryCue !== null
    || runtime.showCancelled || runtime.showHotkeyMiss
    || runtime.showMicrophoneFailure !== null || runtime.modelUnavailable !== null;
  const activity = status === 'recording' ? 'recording'
    : status !== 'idle' || showDone || cueActive ? 'processing' : 'idle';
  // The idle floating pill is invisible and click-through, so hover must not
  // open a card over whatever sits above the Dock.
  const floatingPill = settingsMirror.overlayPlacement === 'floating_pill';
  const pillHidden = floatingPill && activity === 'idle';
  // The pill rests at one width per setting, so dictating never moves it; only
  // a speech-model warning widens it, and it narrows again when that clears.
  const modelCueActive = runtime.modelUnavailable !== null;
  const pillSize = overlayPillSize(
    settingsMirror.overlayPlacement, modelCueActive, settingsMirror.overlayLiveWords);
  // The floating pill carries the words itself; the notch keeps its own window.
  const liveWordsText = useDictationPartial(floatingPill && settingsMirror.overlayLiveWords,
    { presentNatively: false });
  // Hovering the pill while dictating unrolls the wheel: once a line has
  // rolled away, the card shows the lines already said instead of the controls.
  const [finishedLines, setFinishedLines] = useState<string[]>([]);
  const showTranscript = floatingPill && settingsMirror.overlayLiveWords && status === 'recording'
    && finishedLines.length > 0 && !showMeetingSuggestion;
  const overlayContent: OverlayContent = showMeetingSuggestion
    ? 'meeting_suggestion'
    : showTranscript ? 'transcript' : 'controls';
  const { phase, expanded, expandedRef, islandRef, onHoverStart, onHoverEnd } =
    useOverlayExpansion({ forcedOpen: showMeetingSuggestion, content: overlayContent,
      activity, placement: settingsMirror.overlayPlacement, pillSize, hoverEnabled: !pillHidden });
  const waveform = useWaveform(status);
  useLiveWordsLift(geometry, expanded);
  // With Live Words the collapsed capsule hugs the front line; the window
  // keeps its size, so this moves only the drawn capsule and its material.
  const [frontW, setFrontW] = useState<number | null>(null);
  const fitW = geometry ? overlayPillFitWidth(geometry.floating, pillSize, geometry.island.pillW,
    status === 'recording' && floatingPill && settingsMirror.overlayLiveWords, frontW) : undefined;
  // Rust shapes the pill's native Bezel material from this same state.
  const material = useOverlayMaterial({
    floating: floatingPill,
    shown: activity !== 'idle', expanded, content: overlayContent, pillW: fitW ?? null,
  });

  const recordingControls = useRecordingControls({
    status, statusRef, disabledRef: runtime.disabledRef, expandedRef,
  });
  const { startRefused } = recordingControls;

  const visual = deriveVisual(
    status,
    runtime.showCancelled,
    runtime.showHotkeyMiss,
    runtime.disabled,
    runtime.showMicrophoneFailure,
    stillConnecting,
    calibrating,
    meetingPhase,
    runtime.showClipboardOnly,
    runtime.modelUnavailable,
    startRefused,
  );

  // While the model cue shows, a click on the pill opens the page that repairs
  // it instead of starting or stopping a recording.
  const { openSettingsPage } = settingsMirror;
  const { confirmModelUnavailable } = runtime;
  const { handleClick: recordingClick } = recordingControls;
  // Re-check the cue against the live runtime first: a model repaired or
  // switched within the 8 s window must not send the click to Settings.
  const onPillClick = useCallback(() => {
    void confirmModelUnavailable().then((stillBlocked) => {
      if (stillBlocked) void openSettingsPage(MODEL_SETTINGS_TARGET);
      else recordingClick();
    });
  }, [confirmModelUnavailable, openSettingsPage, recordingClick]);

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
    if (status === 'idle' && !meetingBusy && !disabled && !calibrating) return;
    meetingSuggestion.clear();
  }, [status, meetingBusy, disabled, calibrating, meetingSuggestion.clear]);

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
        tapMissed={visual.showTapMissedLabel || modelCueActive || startRefused}
        material={material}
        fitW={fitW}
        onDoubleClick={calibrating || meetingBusy || showMeetingSuggestion || modelCueActive ? undefined : recordingControls.handleDoubleClick}
        onClick={calibrating || meetingBusy || showMeetingSuggestion ? undefined : modelCueActive ? onPillClick : recordingControls.handleClick}
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
          done={showDone}
          liveWords={floatingPill && settingsMirror.overlayLiveWords ? liveWordsText : null}
          // Unrolled, the row spans the card so the front line keeps the x of
          // the lines above it.
          fitW={expanded && showTranscript ? geometry.island.pillW : fitW}
          onFrontWidth={setFrontW}
          onFinishedLines={setFinishedLines}
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
        ) : showTranscript ? (
          <OverlayTranscript geometry={geometry} expanded={expanded} lines={finishedLines} />
        ) : (
          <OverlayDropdown
            geometry={geometry}
            expanded={expanded}
            status={status}
            stillConnecting={stillConnecting}
            showTapMissed={visual.showTapMissedLabel}
            startRefused={startRefused && !visual.showTapMissedLabel}
            modelLabel={runtime.modelUnavailable ? modelUnavailableWord(runtime.modelUnavailable) : null}
            smartDestinationStatus={runtime.smartDestinationStatus}
            smartDestinationAppName={runtime.smartDestinationAppName}
            disabled={runtime.disabled}
            autoPaste={settingsMirror.autoPaste}
            fileOutputEnabled={settingsMirror.fileOutputEnabled}
            onToggleDisabled={settingsMirror.handleToggleDisabled}
            onToggleAutoPaste={settingsMirror.handleToggleAutoPaste}
            onOpenSettings={settingsMirror.handleOpenSettings}
          />
        )}
      </OverlayIsland>
    </div>
  );
}
