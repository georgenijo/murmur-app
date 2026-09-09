import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { AudioDeviceDescriptor, AudioInputLidState } from '../../lib/audioDevices';
import {
  audioDeviceSelectOptions,
  followSystemDefaultOptionLabel,
  previewSmartAutoSelection,
} from '../../lib/audioDevices';
import type { Settings } from '../../lib/settings';
import {
  retrySmartAutoProbe,
  smartAutoMicrophoneReasonLabel,
  smartAutoProbePhaseLabel,
} from '../../lib/smartAutoMicrophone';
import { useSmartAutoMicrophoneStatus } from '../../lib/hooks/useSmartAutoMicrophoneStatus';
import {
  cancelMicrophonePreview,
  getMicrophonePreviewStatus,
  IDLE_MICROPHONE_PREVIEW,
  microphoneClassificationLabel,
  microphoneLevelPercent,
  microphonePeakPercent,
  microphoneSignalVerificationLabel,
  smoothMicrophoneMeterValue,
  startMicrophonePreview,
  stopMicrophonePreview,
  updateMicrophonePreviewVadSensitivity,
  verifyMicrophonePreviewSignal,
  type MicrophonePreviewLevel,
  type MicrophonePreviewStatus,
  type MicrophonePreviewVad,
  type MicrophonePreviewVadDecision,
  type MicrophoneSignalClassification,
} from '../../lib/microphonePreview';
import { Select } from '../ui/Select';
import { useSettingsSurfaceActive } from './SettingsSurfaceContext';
import AnimatedSwitch from '../ui/animated-switch/animated-switch';

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
  smartAuto?: Pick<Settings,
    'smartAutoMicrophoneEnabled'
    | 'smartAutoProbeEnabled'
    | 'smartAutoApprovedDeviceIds'
    | 'smartAutoPreferredDeviceIds'
    | 'smartAutoAllowContinuity'>;
  lidState?: AudioInputLidState;
  onSmartAutoChange?: (updates: Partial<Settings>) => void;
}

function levelColor(classification: MicrophoneSignalClassification): string {
  if (classification === 'clipping') return 'bg-error';
  if (classification === 'signal_detected') return 'bg-success';
  if (classification === 'too_quiet') return 'bg-warning';
  return 'bg-on-surface-variant/35';
}

interface MicrophonePickerProps {
  microphone: string;
  devices: AudioDeviceDescriptor[];
  defaultInputId: string | null;
  disabled: boolean;
  smartAuto: NonNullable<MicrophoneInputTestProps['smartAuto']>;
  smartAutoSelection: ReturnType<typeof previewSmartAutoSelection>;
  lidState: AudioInputLidState;
  onSelectManual: (deviceId: string) => void;
  onSmartAutoChange: NonNullable<MicrophoneInputTestProps['onSmartAutoChange']>;
  describedBy?: string;
}

function microphoneAvailabilityReason(device: AudioDeviceDescriptor, defaultInputId: string | null, lidState: AudioInputLidState, allowContinuity: boolean): string {
  if (!device.hasInput) return 'Not an input device';
  if (!device.connected) return 'Not connected';
  if (device.kind === 'continuity' && !allowContinuity) return 'Allow iPhone microphones below to use it';
  if (device.kind === 'builtIn' && lidState === 'closed') return 'Unavailable while the MacBook lid is closed';
  if (device.kind === 'builtIn' && lidState === 'unknown') return 'Waiting for MacBook lid status';
  if (device.id === defaultInputId) return 'macOS default';
  if (device.kind === 'continuity') return 'iPhone Continuity Camera';
  if (device.kind === 'external') return 'External microphone';
  if (device.kind === 'builtIn') return 'Available when preferred or set as the macOS default';
  return 'Available';
}

function MicrophonePicker({ microphone, devices, defaultInputId, disabled, smartAuto, smartAutoSelection, lidState, onSelectManual, onSmartAutoChange, describedBy }: MicrophonePickerProps) {
  const [open, setOpen] = useState(false);
  const pickerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const restoreFocusWhenEnabledRef = useRef(false);
  const smartAutoActive = smartAuto.smartAutoMicrophoneEnabled;
  const manualDevices = devices.filter((device) => device.hasInput);
  const manualOptions = audioDeviceSelectOptions(manualDevices);
  const deviceLabel = (device: AudioDeviceDescriptor) => (
    manualOptions.find((option) => option.value === device.id)?.label ?? device.name
  );
  const approvalDevices = manualDevices.filter((device) => device.kind !== 'unknown');
  const knownIds = new Set(approvalDevices.map((device) => device.id));
  const unavailableApprovedIds = smartAuto.smartAutoApprovedDeviceIds.filter((id) => !knownIds.has(id));
  const selectedLabel = smartAutoActive
    ? smartAutoSelection ? `Smart Auto · Available: ${smartAutoSelection.device.name}` : 'Smart Auto · No preview candidate'
    : microphone === 'system_default'
      ? followSystemDefaultOptionLabel(manualDevices, defaultInputId)
      : manualOptions.find((device) => device.value === microphone)?.label ?? 'Microphone unavailable';

  useEffect(() => {
    if (disabled) {
      setOpen(false);
      return;
    }
    if (restoreFocusWhenEnabledRef.current) {
      restoreFocusWhenEnabledRef.current = false;
      if (document.activeElement === document.body) triggerRef.current?.focus();
    }
  }, [disabled]);

  useEffect(() => {
    if (!open) return;
    pickerRef.current?.querySelector<HTMLInputElement>('input[type="radio"]:checked')?.focus();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const closeOnOutsideClick = (event: MouseEvent) => {
      if (event.target instanceof Node && !pickerRef.current?.contains(event.target)) setOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener('mousedown', closeOnOutsideClick);
    document.addEventListener('keydown', closeOnEscape);
    return () => {
      document.removeEventListener('mousedown', closeOnOutsideClick);
      document.removeEventListener('keydown', closeOnEscape);
    };
  }, [open]);

  const setApproved = (deviceId: string, approved: boolean) => {
    const approvedIds = approved
      ? Array.from(new Set([...smartAuto.smartAutoApprovedDeviceIds, deviceId]))
      : smartAuto.smartAutoApprovedDeviceIds.filter((id) => id !== deviceId);
    onSmartAutoChange({
      smartAutoApprovedDeviceIds: approvedIds,
      smartAutoPreferredDeviceIds: smartAuto.smartAutoPreferredDeviceIds.filter((id) => approvedIds.includes(id)),
    });
  };
  const setPreferred = (deviceId: string) => {
    onSmartAutoChange({
      smartAutoPreferredDeviceIds: [
        deviceId,
        ...smartAuto.smartAutoPreferredDeviceIds.filter((id) => id !== deviceId && smartAuto.smartAutoApprovedDeviceIds.includes(id)),
      ],
    });
  };
  const selectManual = (deviceId: string) => {
    if (pickerRef.current?.contains(document.activeElement)) {
      restoreFocusWhenEnabledRef.current = true;
    }
    onSelectManual(deviceId);
  };

  return (
    <div
      ref={pickerRef}
      className="relative"
      onBlur={(event) => {
        if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
    >
      <button ref={triggerRef} type="button" aria-label="Microphone input" aria-describedby={describedBy} aria-haspopup="dialog" aria-expanded={open} disabled={disabled} onClick={() => setOpen((current) => !current)} className="flex h-8 w-full items-center justify-between rounded-(--ui-radius-control) border border-(--ui-hairline) bg-(--ui-tint-raised) px-3 text-left text-sm text-on-surface shadow-(--ui-shadow-1) transition-colors focus:border-transparent focus:outline-none focus:ring-2 focus:ring-primary disabled:cursor-not-allowed disabled:opacity-50">
        <span className="truncate">{selectedLabel}</span>
        <svg className={`ml-2 h-4 w-4 shrink-0 text-on-surface-variant transition-transform ${open ? 'rotate-180' : ''}`} fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" /></svg>
      </button>
      {open && (
        <div role="dialog" aria-label="Choose microphone mode and Smart Auto microphones" className="absolute z-20 mt-1 max-h-[26rem] w-full overflow-auto rounded-(--ui-radius-popover) border border-(--ui-hairline-strong) bg-surface-container-lowest p-2 shadow-(--ui-shadow-3)">
          <fieldset>
            <legend className="px-2 py-1 text-[11px] font-semibold uppercase tracking-wider text-on-surface-variant">Input mode</legend>
            <label className="settings-microphone-choice">
              <input type="radio" name="microphone-mode" checked={smartAutoActive} disabled={disabled} onChange={() => onSmartAutoChange({ smartAutoMicrophoneEnabled: true })} />
              <span><span className="block font-medium">Smart Auto</span><span className="block text-xs text-on-surface-variant">Choose from microphones you allow</span></span>
            </label>
            <label className="settings-microphone-choice">
              <input type="radio" name="microphone-mode" checked={!smartAutoActive && microphone === 'system_default'} disabled={disabled} onChange={() => selectManual('system_default')} />
              <span><span className="block font-medium">Follow macOS Default</span><span className="block text-xs text-on-surface-variant">{manualDevices.find((device) => device.id === defaultInputId)?.name ?? 'No default reported'}</span></span>
            </label>
            {manualDevices.map((device) => (
              <label key={`manual-${device.id}`} className="settings-microphone-choice">
                <input type="radio" name="microphone-mode" checked={!smartAutoActive && microphone === device.id} disabled={disabled} onChange={() => selectManual(device.id)} />
                <span><span className="block font-medium">{deviceLabel(device)}</span><span className="block text-xs text-on-surface-variant">Use only this microphone</span></span>
              </label>
            ))}
          </fieldset>
          {smartAutoActive && (
            <fieldset className="mt-2 border-t border-outline-variant/20 pt-2">
              <legend className="px-2 py-1 text-[11px] font-semibold uppercase tracking-wider text-on-surface-variant">Allowed for Smart Auto</legend>
              {approvalDevices.map((device) => {
                const approved = smartAuto.smartAutoApprovedDeviceIds.includes(device.id);
                const previewCandidate = smartAutoSelection?.device.id === device.id;
                const preferred = smartAuto.smartAutoPreferredDeviceIds[0] === device.id;
                return (
                  <div key={`approved-${device.id}`} className="settings-microphone-choice">
                    <label className="settings-microphone-choice-main">
                      <input type="checkbox" checked={approved} disabled={disabled} onChange={(event) => setApproved(device.id, event.target.checked)} />
                      <span className="min-w-0 flex-1">
                        <span className="flex items-center gap-2"><span className="truncate font-medium">{deviceLabel(device)}</span>{previewCandidate && <span className="settings-microphone-active-badge">Available</span>}</span>
                        <span className="block text-xs text-on-surface-variant">{microphoneAvailabilityReason(device, defaultInputId, lidState, smartAuto.smartAutoAllowContinuity)}</span>
                      </span>
                    </label>
                    <button type="button" disabled={disabled || !approved} aria-label={`Prefer ${deviceLabel(device)} for Smart Auto`} aria-pressed={preferred} onClick={() => setPreferred(device.id)} className="settings-microphone-preference">
                      {preferred ? 'Preferred' : 'Prefer'}
                    </button>
                  </div>
                );
              })}
              {unavailableApprovedIds.map((id, index) => (
                <label key={id} className="settings-microphone-choice">
                  <input type="checkbox" checked disabled={disabled} onChange={() => setApproved(id, false)} />
                  <span><span className="block font-medium">Unavailable microphone {index + 1}</span><span className="block text-xs text-on-surface-variant">Previously approved. Uncheck to forget it.</span></span>
                </label>
              ))}
              <label className="settings-microphone-choice mt-1 border-t border-outline-variant/15 pt-2">
                <AnimatedSwitch size="sm" aria-label="Allow approved iPhone Continuity Camera microphones" checked={smartAuto.smartAutoAllowContinuity} disabled={disabled} onCheckedChange={(checked) => onSmartAutoChange({ smartAutoAllowContinuity: checked })} />
                <span><span className="block font-medium">Allow iPhone microphones</span><span className="block text-xs text-on-surface-variant">Only approved Continuity Camera inputs</span></span>
              </label>
              <label className="settings-microphone-choice">
                <AnimatedSwitch size="sm" aria-label="Check approved microphones in the background" checked={smartAuto.smartAutoProbeEnabled} disabled={disabled} onCheckedChange={(checked) => onSmartAutoChange({ smartAutoProbeEnabled: checked })} />
                <span>
                  <span className="block font-medium">Background signal checks</span>
                  <span className="block text-xs text-on-surface-variant">Briefly checks approved inputs while Murmur is idle. Audio is never transcribed or saved.</span>
                </span>
              </label>
            </fieldset>
          )}
        </div>
      )}
    </div>
  );
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
  smartAuto,
  lidState = 'unknown',
  onSmartAutoChange,
}: MicrophoneInputTestProps) {
  const surfaceActive = useSettingsSurfaceActive();
  const selectorHelperId = useId();
  const monitoringActive = active && surfaceActive;
  const [status, setStatus] = useState<MicrophonePreviewStatus>(IDLE_MICROPHONE_PREVIEW);
  const [operation, setOperation] = useState<'idle' | 'starting' | 'switching'>('idle');
  const [previewDevice, setPreviewDevice] = useState<string | null>(null);
  const [autoStartSuspended, setAutoStartSuspended] = useState(false);
  const [subscriptionsReady, setSubscriptionsReady] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [probeRetryError, setProbeRetryError] = useState<string | null>(null);
  const [verification, setVerification] = useState<{
    previewId: number;
    configurationKey: string;
    message: string;
    pending: boolean;
  } | null>(null);
  const verificationPendingRef = useRef(false);
  const [vadDecision, setVadDecision] = useState<MicrophonePreviewVadDecision | 'listening'>('listening');
  const statusRef = useRef(status);
  const mountedRef = useRef(true);
  const operationRef = useRef<Promise<void> | null>(null);
  const vadUpdateRef = useRef<Promise<void>>(Promise.resolve());
  const eventVersionRef = useRef(0);
  const refreshSmartAutoStatusRef = useRef<() => Promise<void>>(async () => {});
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
    if (next.state === 'error') {
      setAutoStartSuspended(true);
      setActionError(next.message ?? 'Microphone preview stopped.');
    }
    if (next.previewId === null) setPreviewDevice(null);
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
          void refreshSmartAutoStatusRef.current();
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
        if (!disposed) {
          setAutoStartSuspended(true);
          setActionError(String(error));
        }
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

  const smartAutoActive = smartAuto?.smartAutoMicrophoneEnabled === true;
  const smartAutoSelection = smartAutoActive && smartAuto
    ? previewSmartAutoSelection({
      approvedDeviceIds: smartAuto.smartAutoApprovedDeviceIds,
      preferredDeviceIds: smartAuto.smartAutoPreferredDeviceIds,
      allowContinuity: smartAuto.smartAutoAllowContinuity,
    }, devices, defaultInputId, lidState)
    : null;
  const smartAutoRequest = useMemo(() => smartAutoActive && smartAuto ? {
    approvedDeviceIds: smartAuto.smartAutoApprovedDeviceIds,
    preferredDeviceIds: smartAuto.smartAutoPreferredDeviceIds,
    allowContinuity: smartAuto.smartAutoAllowContinuity,
  } : null, [
    smartAutoActive,
    smartAuto?.smartAutoAllowContinuity,
    smartAuto?.smartAutoApprovedDeviceIds,
    smartAuto?.smartAutoPreferredDeviceIds,
  ]);
  const { view: smartAutoStatus, refresh: refreshSmartAutoStatus } = useSmartAutoMicrophoneStatus(
    smartAutoRequest,
    monitoringActive,
  );
  refreshSmartAutoStatusRef.current = refreshSmartAutoStatus;
  const previewMicrophone = smartAutoSelection?.device.id ?? microphone;
  const smartAutoUnavailable = smartAutoActive && smartAutoSelection === null;
  const start = useCallback(() => runExclusive(async () => {
    setOperation('starting');
    setActionError(null);
    setPreviewDevice(previewMicrophone);
    try {
      const next = await startMicrophonePreview(
        previewMicrophone,
        vadSensitivity,
        smartAutoRequest,
      );
      if (!mountedRef.current) {
        if (next.previewId !== null) void cancelMicrophonePreview(next.previewId).catch(() => {});
        return;
      }
      applyStatus(next);
    } catch (error) {
      if (mountedRef.current) {
        setPreviewDevice(null);
        setAutoStartSuspended(true);
        setActionError(String(error));
      }
    }
  }), [applyStatus, previewMicrophone, runExclusive, smartAutoRequest, vadSensitivity]);

  const previewConfigurationKey = useMemo(() => JSON.stringify({
    defaultInputId,
    microphone,
    previewMicrophone,
    smartAuto: smartAutoRequest,
  }), [defaultInputId, microphone, previewMicrophone, smartAutoRequest]);
  const previewConfigurationKeyRef = useRef(previewConfigurationKey);
  const smartAutoActiveRef = useRef(smartAutoActive);
  previewConfigurationKeyRef.current = previewConfigurationKey;
  smartAutoActiveRef.current = smartAutoActive;
  const previousPreviewConfigurationRef = useRef(previewConfigurationKey);
  const previousMonitoringActiveRef = useRef(monitoringActive);

  useEffect(() => {
    const configurationChanged = previousPreviewConfigurationRef.current !== previewConfigurationKey;
    const monitoringReentered = monitoringActive && !previousMonitoringActiveRef.current;
    previousPreviewConfigurationRef.current = previewConfigurationKey;
    previousMonitoringActiveRef.current = monitoringActive;
    if (configurationChanged || monitoringReentered) {
      setAutoStartSuspended(false);
      setActionError(null);
    }
  }, [monitoringActive, previewConfigurationKey]);

  useEffect(() => {
    if (!subscriptionsReady) return;
    if (!monitoringActive || !ready || dictationBusy || missingDevice || smartAutoUnavailable || !inventoryAvailable) {
      const previewId = statusRef.current.previewId;
      if (previewId !== null) void cancelMicrophonePreview(previewId).catch(() => {});
      return;
    }
    const previewId = statusRef.current.previewId;
    if (previewId === null) {
      if (!autoStartSuspended) void start();
      return;
    }
    if (previewDevice !== null && previewDevice !== previewMicrophone) {
      void runExclusive(async () => {
        setOperation('switching');
        setActionError(null);
        try {
          applyStatus(await stopMicrophonePreview(previewId));
        } catch (error) {
          if (mountedRef.current) {
            setAutoStartSuspended(true);
            setActionError(String(error));
          }
        }
      });
    }
  }, [
    applyStatus,
    autoStartSuspended,
    dictationBusy,
    inventoryAvailable,
    microphone,
    missingDevice,
    monitoringActive,
    previewDevice,
    previewMicrophone,
    ready,
    runExclusive,
    smartAutoUnavailable,
    start,
    status.previewId,
    subscriptionsReady,
  ]);

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
        if (mountedRef.current) {
          setAutoStartSuspended(true);
          setActionError(String(error));
        }
        return;
      }
      onChange(nextMicrophone);
    });
  }, [applyStatus, onChange, runExclusive]);

  const ownsPreview = status.previewId !== null;
  const verifySignal = async () => {
    const previewId = statusRef.current.previewId;
    if (previewId === null || verificationPendingRef.current) return;
    const configurationKey = previewConfigurationKeyRef.current;
    const stillOwnsVerification = () => mountedRef.current
      && statusRef.current.previewId === previewId
      && previewConfigurationKeyRef.current === configurationKey;
    verificationPendingRef.current = true;
    setVerification({ previewId, configurationKey, pending: true, message: 'Speak normally for five seconds. Checking for one second of sustained signal…' });
    try {
      const result = await verifyMicrophonePreviewSignal(previewId);
      if (stillOwnsVerification()) {
        setVerification({ previewId, configurationKey, pending: false, message: microphoneSignalVerificationLabel(result) });
      }
    } catch (error) {
      if (stillOwnsVerification()) {
        setVerification({ previewId, configurationKey, pending: false, message: String(error) });
      }
    } finally {
      verificationPendingRef.current = false;
      if (mountedRef.current) {
        setVerification((current) => current?.previewId === previewId
          && current.configurationKey === configurationKey
          && current.pending
          ? null
          : current);
      }
      if (stillOwnsVerification() && smartAutoActiveRef.current) {
        await refreshSmartAutoStatusRef.current();
      }
    }
  };
  const busy = operation !== 'idle';
  const retryPreview = () => {
    if (statusRef.current.previewId !== null) return;
    setActionError(null);
    applyStatus(IDLE_MICROPHONE_PREVIEW);
    setAutoStartSuspended(false);
  };
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
  const deviceOptions = audioDeviceSelectOptions(devices);
  const previewCandidateLabel = smartAutoSelection
    ? deviceOptions.find((device) => device.value === smartAutoSelection.device.id)?.label
      ?? smartAutoSelection.device.name
    : null;
  const previewDeviceLabel = previewDevice
    ? deviceOptions.find((device) => device.value === previewDevice)?.label
      ?? (previewDevice === 'system_default' ? 'macOS default' : 'selected microphone')
    : null;
  const verifiedStatus = smartAutoStatus.kind === 'resolved' && smartAutoStatus.status.state === 'ready'
    ? smartAutoStatus.status
    : null;
  const probingStatus = smartAutoStatus.kind === 'resolved' && smartAutoStatus.status.state === 'probing'
    ? smartAutoStatus.status
    : null;
  const verifiedDeviceLabel = verifiedStatus
    ? deviceOptions.find((device) => device.value === verifiedStatus.deviceId)?.label
      ?? 'verified microphone'
    : null;
  const probingDeviceLabel = probingStatus
    ? deviceOptions.find((device) => device.value === probingStatus.deviceId)?.label
      ?? 'approved microphone'
    : null;
  const automaticHelperText = smartAutoActive
    ? null
    : microphone === 'system_default' && inventoryAvailable
    ? defaultDevice
      ? `Following macOS: ${defaultDevice.name}. Docking, undocking, or changing the system input applies automatically to the next recording.`
      : 'macOS does not currently report a default microphone. Murmur will follow one when it becomes available.'
    : null;
  const describedBy = selectorHelperText || automaticHelperText || smartAutoActive
    ? selectorHelperId
    : undefined;

  return (
    <div>
      <label className="mb-2 block text-sm font-medium text-on-surface">Microphone</label>
      {smartAuto && onSmartAutoChange ? (
        <MicrophonePicker
          microphone={microphone}
          devices={devices}
          defaultInputId={defaultInputId}
          disabled={busy || !inventoryAvailable}
          smartAuto={smartAuto}
          smartAutoSelection={smartAutoSelection}
          lidState={lidState}
          describedBy={describedBy}
          onSmartAutoChange={onSmartAutoChange}
          onSelectManual={(next) => {
            if (smartAutoActive) onSmartAutoChange({ smartAutoMicrophoneEnabled: false });
            switchDevice(next);
          }}
        />
      ) : (
        <Select
          value={microphone}
          onChange={switchDevice}
          disabled={busy || !inventoryAvailable}
          aria-label="Microphone input"
          aria-describedby={describedBy}
          items={[
            { value: 'system_default', label: followSystemDefaultOptionLabel(devices, defaultInputId) },
            ...audioDeviceSelectOptions(devices.filter((device) => device.hasInput)),
          ]}
        />
      )}
      {selectorHelperText && missingDevice && inventoryAvailable ? (
        <p
          id={selectorHelperId}
          className="mt-2 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-on-surface"
        >
          {selectorHelperText}
        </p>
      ) : smartAutoActive ? (
        <div id={selectorHelperId} className="mt-2 rounded-lg border border-outline-variant/25 bg-surface-container-low px-3 py-2 text-xs text-on-surface-variant" aria-live="polite">
          <p>
            <span className="font-medium text-on-surface">Availability candidate: </span>
            {previewCandidateLabel
              ? `${previewCandidateLabel}. Chosen from current availability.`
              : 'None. Approve an available microphone or choose a fixed input.'}
          </p>
          {previewDeviceLabel && (
            <p className="mt-1"><span className="font-medium text-on-surface">Previewing now: </span>{previewDeviceLabel}.</p>
          )}
          <p className="mt-1">Background checks are {smartAuto?.smartAutoProbeEnabled ? 'on' : 'off'}.</p>
          {smartAutoStatus.kind === 'loading' ? (
            <p className="mt-1">Checking recent signal evidence for the next capture…</p>
          ) : probingStatus ? (
            <p className="mt-1 text-primary">
              <span className="font-medium">Background check: </span>
              {probingDeviceLabel}, {smartAutoProbePhaseLabel(probingStatus.phase)}. This brief signal check is not transcribed or saved.
            </p>
          ) : smartAutoStatus.kind === 'resolved' && smartAutoStatus.status.state === 'ready' ? (
            <p className="mt-1 text-success">
              <span className="font-medium">Next capture ready: </span>
              {verifiedDeviceLabel}. Verified recently, {smartAutoMicrophoneReasonLabel(smartAutoStatus.status.reason)}.
            </p>
          ) : smartAutoStatus.kind === 'resolved' && smartAutoStatus.status.state === 'blocked' ? (
            <>
              <p className="mt-1 text-warning"><span className="font-medium">Next capture blocked: </span>{smartAutoStatus.status.message}</p>
              {smartAutoStatus.status.retryAfterMs !== null && (
                <p className="mt-1">Auto cooldown: about {Math.max(1, Math.ceil(smartAutoStatus.status.retryAfterMs / 1000))} seconds remaining. Another check also requires Murmur to be idle.</p>
              )}
              <p className="mt-1">Verify the preview candidate below, or choose a fixed microphone.</p>
              {smartAuto?.smartAutoProbeEnabled && (
                <button
                  type="button"
                  className="mt-2 rounded border border-outline-variant px-2 py-1 text-on-surface"
                  onClick={() => {
                    setProbeRetryError(null);
                    void retrySmartAutoProbe()
                      .then(() => refreshSmartAutoStatusRef.current())
                      .catch(() => setProbeRetryError('Could not request another background check. Try again after the current cooldown.'));
                  }}
                >
                  Retry background checks
                </button>
              )}
              {probeRetryError && <p className="mt-1 text-error" role="alert">{probeRetryError}</p>}
            </>
          ) : smartAutoStatus.kind === 'unavailable' ? (
            <>
              <p className="mt-1 text-warning">Murmur could not confirm a safe microphone for the next capture.</p>
              <p className="mt-1">Verify the preview candidate below, or choose a fixed microphone.</p>
            </>
          ) : null}
          {previewCandidateLabel && (
            <button
              type="button"
              className="mt-2 rounded border border-outline-variant px-2 py-1 text-on-surface disabled:opacity-50"
              disabled={busy}
              onClick={() => {
                onSmartAutoChange?.({ smartAutoMicrophoneEnabled: false });
                switchDevice(previewMicrophone);
              }}
            >
              Use {previewCandidateLabel} only
            </button>
          )}
        </div>
      ) : selectorHelperText || automaticHelperText ? (
        <p id={selectorHelperId} className="mt-2 text-xs text-on-surface-variant">
          {selectorHelperText ?? automaticHelperText}
        </p>
      ) : (
        null
      )}
      {smartAutoUnavailable && <p className="mt-2 text-xs text-warning">No approved microphone is usable right now.</p>}
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
        {autoStartSuspended && status.previewId === null && monitoringActive && (
          <button
            type="button"
            className="mt-2 rounded border border-outline-variant px-2 py-1 text-xs text-on-surface disabled:opacity-50"
            disabled={busy || dictationBusy || !ready || !inventoryAvailable}
            onClick={retryPreview}
          >
            Retry microphone preview
          </button>
        )}
        <div className="mt-2 text-xs text-on-surface-variant">
          <button type="button" className="rounded border border-outline-variant px-2 py-1 text-on-surface disabled:opacity-50" disabled={status.state !== 'active' || busy || dictationBusy || verification?.pending === true} onClick={() => void verifySignal()}>
            {smartAutoActive ? 'Verify preview candidate for 5 seconds' : 'Verify signal for 5 seconds'}
          </button>
          {smartAutoActive && <p className="mt-2">The preview follows availability. Smart Auto requires a recent successful check before it can use this candidate.</p>}
          {verification?.previewId === status.previewId
            && verification.configurationKey === previewConfigurationKey
            && <p className="mt-2" role="status">{verification.message}</p>}
        </div>
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
