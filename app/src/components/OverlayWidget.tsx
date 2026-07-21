import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../lib/log';
import { isDictationStatus } from '../lib/types';
import type { DictationStatus } from '../lib/types';
import { useOverlayGeometry } from '../lib/hooks/useOverlayGeometry';
import { useOverlayExpansion } from '../lib/hooks/useOverlayExpansion';
import { useOverlayRuntime } from '../lib/hooks/useOverlayRuntime';
import { useOverlaySettingsMirror } from '../lib/hooks/useOverlaySettingsMirror';
import { useRecordingControls } from '../lib/hooks/useRecordingControls';
import { useWaveform } from '../lib/hooks/useWaveform';
import { OVERLAY_ISLAND_TRANSITION } from '../lib/overlayMotion';
import { deriveVisual } from './overlay/deriveVisual';
import { OverlayPill } from './overlay/OverlayPill';
import { OverlayDropdown } from './overlay/OverlayDropdown';
import { NotchCalibrationBand } from './overlay/NotchCalibrationBand';

export function OverlayWidget() {
  const geometry = useOverlayGeometry();

  // Shared mutable state written synchronously by both useOverlayRuntime's
  // Tauri listeners and useOverlaySettingsMirror's applySettingsSnapshot.
  // Created here (rather than inside either hook) because each hook needs to
  // write into it and neither can be constructed from the other's return
  // value without an artificial call-order dependency — see the doc comments
  // on useOverlayRuntime / useOverlaySettingsMirror.
  const [disabled, setDisabled] = useState(false);
  const [showHotkeyMiss, setShowHotkeyMiss] = useState(false);
  const [status, setStatus] = useState<DictationStatus>('idle');
  // True while the local-LLM transform is thinking (issue #312 PR-C2). Driven
  // by the broadcast `transform-state-changed` event; the overlay is a
  // separate webview so it listens directly.
  const [transforming, setTransforming] = useState(false);
  const hotkeyMissFeedbackRef = useRef(false);
  const statusRef = useRef<DictationStatus>('idle');

  const settingsMirror = useOverlaySettingsMirror({ setDisabled, setShowHotkeyMiss, hotkeyMissFeedbackRef });

  const runtime = useOverlayRuntime({
    status, statusRef, disabled, setDisabled, showHotkeyMiss, setShowHotkeyMiss, hotkeyMissFeedbackRef,
  });

  // The expansion controller owns the entire expand/collapse + surface lifecycle:
  // dwell/collapse/shrink timers, the serialized set_overlay_expanded writer, and
  // the single cursor poller. It is the only writer to the native resize path.
  const { phase, expanded, expandedRef, islandRef, onHoverStart, onHoverEnd } =
    useOverlayExpansion();

  const waveform = useWaveform(status);

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
  );

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

  // Subscribe to recording status events from Rust. This is the overlay's only
  // status source now that the live-preview hook (which used to carry it) is gone.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<unknown>('recording-status-changed', (event) => {
      if (isDictationStatus(event.payload)) {
        setStatus(event.payload);
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Keep statusRef in sync for the hooks that need synchronous reads (click
  // handlers, the hotkey-tap-rejected listener) rather than a render-time value.
  useEffect(() => {
    statusRef.current = status;
    flog.info('overlay', 'status changed', { status });
  }, [status]);

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

  return (
    <div
      className="w-full h-full flex"
      style={{ background: 'transparent' }}
      onMouseDown={recordingControls.handleMouseDown}
      onDoubleClick={recordingControls.handleDoubleClick}
      onClick={recordingControls.handleClick}
      onMouseEnter={onHoverStart}
      onMouseMove={onHoverStart}
    >
      {/* Dynamic Island: top bar matches notch height; hover expands it downward
          to reveal the quick-settings dropdown. Idle/recording only changes the
          top bar — the dropdown row is identical. */}
      <div
        ref={islandRef}
        className="overlay-island cursor-pointer select-none overflow-hidden"
        onMouseEnter={onHoverStart}
        onMouseMove={onHoverStart}
        onMouseLeave={onHoverEnd}
        style={{
          position: 'relative',
          borderRadius: '0 0 12px 12px',
          // One constant island width in every state — the island IS the window.
          // Only height animates (see OVERLAY_ISLAND_TRANSITION).
          width: geometry.pillActiveW,
          height: topH + (expanded ? geometry.dropdownH : 0),
          marginLeft: geometry.pillMarginActive,
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
        />
        <NotchCalibrationBand geometry={geometry} />
        <OverlayDropdown
          geometry={geometry}
          expanded={expanded}
          status={status}
          showTapMissed={visual.showTapMissedLabel}
          disabled={runtime.disabled}
          autoPaste={settingsMirror.autoPaste}
          fileOutputEnabled={settingsMirror.fileOutputEnabled}
          onToggleDisabled={settingsMirror.handleToggleDisabled}
          onToggleAutoPaste={settingsMirror.handleToggleAutoPaste}
          onOpenSettings={settingsMirror.handleOpenSettings}
        />
      </div>
    </div>
  );
}
