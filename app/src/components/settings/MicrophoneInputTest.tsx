import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { AudioDeviceDescriptor } from '../../lib/audioDevices';
import { audioDeviceSelectOptions } from '../../lib/audioDevices';
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
  type MicrophonePreviewLevel,
  type MicrophonePreviewStatus,
  type MicrophoneSignalClassification,
} from '../../lib/microphonePreview';
import { Select } from '../ui/Select';
import { useSettingsSurfaceActive } from './SettingsSurfaceContext';

interface MicrophoneInputTestProps {
  microphone: string;
  devices: AudioDeviceDescriptor[];
  active: boolean;
  ready: boolean;
  dictationBusy: boolean;
  missingDevice: boolean;
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
  active,
  ready,
  dictationBusy,
  missingDevice,
  onChange,
}: MicrophoneInputTestProps) {
  const surfaceActive = useSettingsSurfaceActive();
  const monitoringActive = active && surfaceActive;
  const [status, setStatus] = useState<MicrophonePreviewStatus>(IDLE_MICROPHONE_PREVIEW);
  const [operation, setOperation] = useState<'idle' | 'starting' | 'switching'>('idle');
  const [subscriptionsReady, setSubscriptionsReady] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const statusRef = useRef(status);
  const mountedRef = useRef(true);
  const operationRef = useRef<Promise<void> | null>(null);
  const eventVersionRef = useRef(0);
  const latestLevelRef = useRef<MicrophonePreviewLevel | null>(null);
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

  useEffect(() => {
    mountedRef.current = true;
    let disposed = false;
    let unlistenStatus: (() => void) | null = null;
    let unlistenLevel: (() => void) | null = null;

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
      const previewId = statusRef.current.previewId;
      if (previewId !== null) void cancelMicrophonePreview(previewId).catch(() => {});
    };
  }, [applyStatus]);

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
      const next = await startMicrophonePreview(microphone);
      if (!mountedRef.current) {
        if (next.previewId !== null) void cancelMicrophonePreview(next.previewId).catch(() => {});
        return;
      }
      applyStatus(next);
    } catch (error) {
      if (mountedRef.current) setActionError(String(error));
    }
  }), [applyStatus, microphone, runExclusive]);

  useEffect(() => {
    if (!subscriptionsReady) return;
    if (!monitoringActive || !ready || dictationBusy || missingDevice) {
      const previewId = statusRef.current.previewId;
      if (previewId !== null) void cancelMicrophonePreview(previewId).catch(() => {});
      return;
    }
    if (statusRef.current.previewId === null) void start();
  }, [dictationBusy, microphone, missingDevice, monitoringActive, ready, start, subscriptionsReady]);

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

  return (
    <div>
      <label className="mb-2 block text-sm font-medium text-on-surface">Microphone</label>
      <Select
        value={microphone}
        onChange={switchDevice}
        disabled={busy}
        aria-label="Microphone input"
        items={[{ value: 'system_default', label: 'System Default' }, ...audioDeviceSelectOptions(devices)]}
      />
      {missingDevice && (
        <p className="mt-2 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-on-surface">
          Selected device not found — choose an available microphone or System Default.
        </p>
      )}
      <div className="mt-3 rounded-lg bg-surface-container-low p-3">
        <div className="flex items-center gap-3">
          <div
            ref={meterRef}
            role="meter"
            aria-label="Live microphone input level"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={0}
            aria-valuetext="Microphone test inactive"
            className="relative h-2.5 min-w-0 flex-1 overflow-hidden rounded-full bg-surface-container-highest"
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
      </div>
    </div>
  );
}
