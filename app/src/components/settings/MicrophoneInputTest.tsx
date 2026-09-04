import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { AudioDeviceDescriptor } from '../../lib/audioDevices';
import {
  audioDeviceSelectOptions,
  followSystemDefaultOptionLabel,
} from '../../lib/audioDevices';
import {
  cancelMicrophonePreview,
  getMicrophonePreviewStatus,
  IDLE_MICROPHONE_PREVIEW,
  microphoneClassificationLabel,
  microphoneLevelPercent,
  microphonePeakPercent,
  smoothMicrophoneMeterValue,
  startMicrophonePreview,
  stopMicrophonePreview,
  updateMicrophonePreviewVadSensitivity,
  type MicrophonePreviewLevel,
  type MicrophonePreviewStatus,
  type MicrophonePreviewVad,
  type MicrophonePreviewVadDecision,
  type MicrophoneSignalClassification,
} from '../../lib/microphonePreview';
import { Select } from '../ui/Select';
import { useSettingsSurfaceActive } from './SettingsSurfaceContext';

interface MicrophoneInputTestProps {
  microphone: string;
  devices: AudioDeviceDescriptor[];
  defaultInputId: string | null;
  active: boolean;
  ready: boolean;
  vadSensitivity: number;
  dictationBusy: boolean;
  missingDevice: boolean;
  inventoryAvailable?: boolean;
  inventoryLoading?: boolean;
  onChange: (microphone: string) => void;
}

function levelColor(classification: MicrophoneSignalClassification): string {
  if (classification === 'clipping') return 'bg-error';
  if (classification === 'signal_detected') return 'bg-success';
  if (classification === 'too_quiet') return 'bg-warning';
  return 'bg-on-surface-variant/35';
}

export function MicrophoneInputTest({
  microphone,
  devices,
  defaultInputId,
  active,
  ready,
  vadSensitivity,
  dictationBusy,
  missingDevice,
  inventoryAvailable = true,
  inventoryLoading = false,
  onChange,
}: MicrophoneInputTestProps) {
  const surfaceActive = useSettingsSurfaceActive();
  const selectorHelperId = useId();
  const monitoringActive = active && surfaceActive;
  const [status, setStatus] = useState<MicrophonePreviewStatus>(IDLE_MICROPHONE_PREVIEW);
  const [operation, setOperation] = useState<'idle' | 'starting' | 'switching'>('idle');
  const [subscriptionsReady, setSubscriptionsReady] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [vadDecision, setVadDecision] = useState<MicrophonePreviewVadDecision | 'listening'>('listening');
  const statusRef = useRef(status);
  const mountedRef = useRef(true);
  const operationRef = useRef<Promise<void> | null>(null);
  const vadUpdateRef = useRef<Promise<void>>(Promise.resolve());
  const eventVersionRef = useRef(0);
  const latestLevelRef = useRef<MicrophonePreviewLevel | null>(null);
  const vadSensitivityRef = useRef(vadSensitivity);
  vadSensitivityRef.current = vadSensitivity;
  const meterRef = useRef<HTMLDivElement>(null);
  const fillRef = useRef<HTMLDivElement>(null);
  const peakRef = useRef<HTMLDivElement>(null);
  const classificationRef = useRef<HTMLSpanElement>(null);
  const paintedClassificationRef = useRef<MicrophoneSignalClassification>('no_signal');
  const displayedLevelRef = useRef(0);
  const displayedPeakRef = useRef(0);
  const lastPaintAtRef = useRef<number | null>(null);
  const lastAccessiblePaintAtRef = useRef<number | null>(null);

  const applyStatus = useCallback((next: MicrophonePreviewStatus) => {
    if (!mountedRef.current) return;
    const currentId = statusRef.current.previewId;
    if (currentId !== null && next.previewId !== null && next.previewId < currentId) return;
    statusRef.current = next;
    setStatus(next);
  }, []);

  const syncVadSensitivity = useCallback((previewId: number, sensitivity: number) => {
    const update = async () => {
      if (
        !mountedRef.current
        || statusRef.current.previewId !== previewId
        || vadSensitivityRef.current !== sensitivity
      ) {
        return;
      }
      try {
        await updateMicrophonePreviewVadSensitivity(previewId, sensitivity);
      } catch {
        if (
          mountedRef.current
          && statusRef.current.previewId === previewId
          && vadSensitivityRef.current === sensitivity
        ) {
          setVadDecision('unavailable');
        }
      }
    };
    vadUpdateRef.current = vadUpdateRef.current.then(update, update);
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    let disposed = false;
    let unlistenStatus: (() => void) | null = null;
    let unlistenLevel: (() => void) | null = null;
    let unlistenVad: (() => void) | null = null;

    void (async () => {
      unlistenStatus = await listen<MicrophonePreviewStatus>(
        'microphone-preview-status',
        (event) => {
          if (disposed) return;
          eventVersionRef.current += 1;
          applyStatus(event.payload);
        },
      );
      unlistenLevel = await listen<MicrophonePreviewLevel>(
        'microphone-preview-level',
        (event) => {
          if (!disposed && event.payload.previewId === statusRef.current.previewId) {
            latestLevelRef.current = event.payload;
          }
        },
      );
      unlistenVad = await listen<MicrophonePreviewVad>(
        'microphone-preview-vad',
        (event) => {
          if (disposed || event.payload.previewId !== statusRef.current.previewId) return;
          if (event.payload.sensitivity !== vadSensitivityRef.current) {
            // A rapid drag can race IPC command delivery. Reassert the latest
            // value so a stale backend decision cannot strand the UI.
            syncVadSensitivity(event.payload.previewId, vadSensitivityRef.current);
            return;
          }
          setVadDecision(event.payload.decision);
        },
      );
      const versionBeforeSnapshot = eventVersionRef.current;
      try {
        const snapshot = await getMicrophonePreviewStatus();
        if (!disposed && eventVersionRef.current === versionBeforeSnapshot) applyStatus(snapshot);
      } catch (error) {
        if (!disposed) setActionError(String(error));
      } finally {
        if (!disposed) setSubscriptionsReady(true);
      }
    })();

    return () => {
      disposed = true;
      mountedRef.current = false;
      unlistenStatus?.();
      unlistenLevel?.();
      unlistenVad?.();
      const previewId = statusRef.current.previewId;
      if (previewId !== null) void cancelMicrophonePreview(previewId).catch(() => {});
    };
  }, [applyStatus, syncVadSensitivity]);

  useEffect(() => {
    let frame = 0;
    const paint = (now: number) => {
      const level = latestLevelRef.current;
      if (level && level.previewId === statusRef.current.previewId) {
        const elapsedMs = lastPaintAtRef.current === null
          ? 1000 / 60
          : now - lastPaintAtRef.current;
        lastPaintAtRef.current = now;
        displayedLevelRef.current = smoothMicrophoneMeterValue(
          displayedLevelRef.current,
          microphoneLevelPercent(level.rms),
          elapsedMs,
        );
        displayedPeakRef.current = smoothMicrophoneMeterValue(
          displayedPeakRef.current,
          microphonePeakPercent(level.peak),
          elapsedMs,
          45,
          450,
        );
        const levelPercent = displayedLevelRef.current;
        const peakPercent = displayedPeakRef.current;
        if (fillRef.current) {
          fillRef.current.style.width = `${levelPercent.toFixed(1)}%`;
          fillRef.current.className = `h-full rounded-full transition-colors ${levelColor(level.classification)}`;
        }
        if (peakRef.current) peakRef.current.style.left = `calc(${peakPercent.toFixed(1)}% - 1px)`;
        if (
          lastAccessiblePaintAtRef.current === null
          || now - lastAccessiblePaintAtRef.current >= 200
        ) {
          lastAccessiblePaintAtRef.current = now;
          const accessibleLevel = Math.round(levelPercent);
          const accessiblePeak = Math.round(peakPercent);
          meterRef.current?.setAttribute('aria-valuenow', String(accessibleLevel));
          meterRef.current?.setAttribute(
            'aria-valuetext',
            `${microphoneClassificationLabel(level.classification)}, level ${accessibleLevel} percent, peak ${accessiblePeak} percent`,
          );
        }
        if (paintedClassificationRef.current !== level.classification) {
          paintedClassificationRef.current = level.classification;
          if (classificationRef.current) {
            classificationRef.current.textContent = microphoneClassificationLabel(level.classification);
          }
        }
      }
      frame = requestAnimationFrame(paint);
    };
    frame = requestAnimationFrame(paint);
    return () => cancelAnimationFrame(frame);
  }, []);

  useEffect(() => {
    setVadDecision('listening');
    if (status.previewId !== null) syncVadSensitivity(status.previewId, vadSensitivity);
  }, [status.previewId, syncVadSensitivity, vadSensitivity]);

  useEffect(() => {
    if (status.previewId !== null) return;
    latestLevelRef.current = null;
    displayedLevelRef.current = 0;
    displayedPeakRef.current = 0;
    lastPaintAtRef.current = null;
    lastAccessiblePaintAtRef.current = null;
    if (fillRef.current) fillRef.current.style.width = '0%';
    if (peakRef.current) peakRef.current.style.left = '0%';
    meterRef.current?.setAttribute('aria-valuenow', '0');
    meterRef.current?.setAttribute('aria-valuetext', 'Microphone test inactive');
    paintedClassificationRef.current = 'no_signal';
    if (classificationRef.current) classificationRef.current.textContent = 'No signal';
    setVadDecision('listening');
  }, [status.previewId]);

  const runExclusive = useCallback(async (task: () => Promise<void>) => {
    if (operationRef.current) return operationRef.current;
    const promise = task().finally(() => {
      operationRef.current = null;
      if (mountedRef.current) setOperation('idle');
    });
    operationRef.current = promise;
    return promise;
  }, []);

  const start = useCallback(() => runExclusive(async () => {
    setOperation('starting');
    setActionError(null);
    try {
      const next = await startMicrophonePreview(microphone, vadSensitivity);
      if (!mountedRef.current) {
        if (next.previewId !== null) void cancelMicrophonePreview(next.previewId).catch(() => {});
        return;
      }
      applyStatus(next);
    } catch (error) {
      if (mountedRef.current) setActionError(String(error));
    }
  }), [applyStatus, microphone, runExclusive, vadSensitivity]);

  useEffect(() => {
    if (!subscriptionsReady) return;
    if (!monitoringActive || !ready || dictationBusy || missingDevice || !inventoryAvailable) {
      const previewId = statusRef.current.previewId;
      if (previewId !== null) void cancelMicrophonePreview(previewId).catch(() => {});
      return;
    }
    if (statusRef.current.previewId === null) void start();
  }, [dictationBusy, inventoryAvailable, microphone, missingDevice, monitoringActive, ready, start, subscriptionsReady]);

  const switchDevice = useCallback((nextMicrophone: string) => {
    void runExclusive(async () => {
      const previewId = statusRef.current.previewId;
      if (previewId === null) {
        onChange(nextMicrophone);
        return;
      }
      setOperation('switching');
      setActionError(null);
      try {
        const stopped = await stopMicrophonePreview(previewId);
        if (!mountedRef.current) return;
        applyStatus(stopped);
      } catch (error) {
        // Keep the user's selection, but never open another device until the
        // previous worker has confirmed teardown.
        onChange(nextMicrophone);
        if (mountedRef.current) setActionError(String(error));
        return;
      }
      onChange(nextMicrophone);
    });
  }, [applyStatus, onChange, runExclusive]);

  const ownsPreview = status.previewId !== null;
  const busy = operation !== 'idle';
  const vadLabel = dictationBusy
    ? 'Paused while recording'
    : vadSensitivity === 0
      ? 'Off · all audio kept'
      : status.state === 'connecting'
        ? 'Starting…'
        : vadDecision === 'speech_detected'
          ? 'Speech detected · kept'
          : vadDecision === 'no_speech'
            ? 'No speech · filtered'
            : vadDecision === 'unavailable'
              ? 'Voice detection unavailable'
              : 'Listening…';
  const showVadDecision = !dictationBusy && vadSensitivity > 0 && status.state === 'active';
  const vadDotClass = showVadDecision && vadDecision === 'speech_detected'
    ? 'bg-success'
    : showVadDecision && vadDecision === 'no_speech'
      ? 'bg-warning'
      : showVadDecision && vadDecision === 'unavailable'
        ? 'bg-error'
        : 'bg-on-surface-variant/45';
  const vadTextClass = showVadDecision && vadDecision === 'speech_detected'
    ? 'text-success'
    : showVadDecision && vadDecision === 'no_speech'
      ? 'text-warning'
      : showVadDecision && vadDecision === 'unavailable'
        ? 'text-error'
        : 'text-on-surface-variant';
  const helperText = actionError ?? status.message ?? (
    dictationBusy
      ? 'Level monitoring pauses while Murmur records and resumes automatically.'
      : !ready
        ? 'Preparing microphone monitoring…'
      : !monitoringActive
        ? 'Level monitoring starts automatically when this page is open.'
        : status.state === 'connecting'
      ? status.stillConnecting ? 'Still connecting. Check macOS microphone access if this continues.' : 'Connecting to the selected microphone…'
      : status.state === 'stopping'
        ? 'Waiting for the microphone worker to close…'
        : ownsPreview
          ? 'Speak normally and watch the live level.'
          : 'Preview stays on this Mac and is never transcribed or saved.'
  );
  const selectorHelperText = !inventoryAvailable
    ? inventoryLoading
      ? 'Loading available microphones…'
      : 'Microphone choices are temporarily unavailable.'
    : missingDevice
      ? 'Selected device not found — choose an available microphone or Follow macOS Default.'
      : null;
  const defaultDevice = devices.find((device) => device.id === defaultInputId) ?? null;
  const automaticHelperText = microphone === 'system_default' && inventoryAvailable
    ? defaultDevice
      ? `Following macOS: ${defaultDevice.name}. Docking, undocking, or changing the system input applies automatically to the next recording.`
      : 'macOS does not currently report a default microphone. Murmur will follow one when it becomes available.'
    : null;
  const describedBy = selectorHelperText || automaticHelperText ? selectorHelperId : undefined;

  return (
    <div>
      <label className="mb-2 block text-sm font-medium text-on-surface">Microphone</label>
      <Select
        value={microphone}
        onChange={switchDevice}
        disabled={busy || !inventoryAvailable}
        aria-label="Microphone input"
        aria-describedby={describedBy}
        items={[
          {
            value: 'system_default',
            label: followSystemDefaultOptionLabel(devices, defaultInputId),
          },
          ...audioDeviceSelectOptions(devices),
        ]}
      />
      {selectorHelperText && missingDevice && inventoryAvailable ? (
        <p
          id={selectorHelperId}
          className="mt-2 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-on-surface"
        >
          {selectorHelperText}
        </p>
      ) : selectorHelperText || automaticHelperText ? (
        <p id={selectorHelperId} className="mt-2 text-xs text-on-surface-variant">
          {selectorHelperText ?? automaticHelperText}
        </p>
      ) : (
        null
      )}
      <div className="settings-meter-card">
        <div className="flex items-center gap-3">
          <div
            ref={meterRef}
            role="meter"
            aria-label="Live microphone input level"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={0}
            aria-valuetext="Microphone test inactive"
            className="settings-meter-track relative h-2.5 min-w-0 flex-1 overflow-hidden rounded-full"
          >
            <div ref={fillRef} className="h-full w-0 rounded-full bg-on-surface-variant/35 transition-colors" />
            <div ref={peakRef} className="absolute inset-y-0 left-0 w-0.5 bg-on-surface" aria-hidden="true" />
          </div>
          <span ref={classificationRef} aria-live="polite" className="w-24 text-right text-xs font-medium text-on-surface">
            No signal
          </span>
        </div>
        <p className={`mt-2 text-xs ${actionError || status.message ? 'text-error' : 'text-on-surface-variant'}`} role={actionError || status.message ? 'alert' : undefined}>
          {helperText}
        </p>
        <div className="mt-2 flex items-center justify-between gap-3 border-t border-outline-variant/15 pt-2 text-xs">
          <span className="text-on-surface-variant">
            Voice detection · {vadSensitivity === 0 ? 'Off' : `${vadSensitivity}%`}
          </span>
          <span className="inline-flex items-center gap-1.5 font-medium" aria-live="polite" aria-atomic="true">
            <span className={`h-1.5 w-1.5 rounded-full ${vadDotClass}`} aria-hidden="true" />
            <span className={vadTextClass}>{vadLabel}</span>
          </span>
        </div>
      </div>
    </div>
  );
}
