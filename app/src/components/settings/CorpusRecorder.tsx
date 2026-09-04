import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  audioDeviceSelectOptions,
  selectedDeviceExists,
  type AudioInputInventoryV1,
} from '../../lib/audioDevices';
import {
  cancelCorpusRecording,
  getCorpusSummary,
  openCorpusFolder,
  PERSONAL_CORPUS_PROMPTS,
  startCorpusRecording,
  stopCorpusRecording,
  type CorpusRecordingEntry,
  type CorpusStatusEvent,
  type CorpusSummary,
} from '../../lib/corpusRecorder';
import type { Settings } from '../../lib/settings';
import type { DictationStatus } from '../../lib/types';

type RecorderPhase = CorpusStatusEvent['state'];

export function corpusMicrophoneAvailability(
  inventory: AudioInputInventoryV1 | null,
  deviceId: string,
) {
  const inventoryAvailable = inventory?.status === 'available';
  const displayDevices = inventory?.devices ?? [];
  const selectableDevices = inventoryAvailable ? displayDevices : [];
  return {
    inventoryAvailable,
    displayDevices,
    selectableDevices,
    deviceSelectable: inventoryAvailable && (
      deviceId === 'system_default'
        ? selectableDevices.length > 0
        : selectedDeviceExists(deviceId, selectableDevices)
    ),
  };
}

function durationLabel(milliseconds: number): string {
  const seconds = Math.max(0, milliseconds) / 1_000;
  return `${seconds.toFixed(seconds >= 10 ? 1 : 2)}s`;
}

function levelPercent(level: number): number {
  return Math.min(100, Math.max(0, level * 650));
}

export function CorpusRecorder({
  status,
  benchmarkRunning,
  fileTranscribing,
  settings,
  audioInventory,
  onUpdateSettings,
  onBusyChange,
}: {
  status: DictationStatus;
  benchmarkRunning: boolean;
  fileTranscribing: boolean;
  settings: Settings;
  audioInventory: AudioInputInventoryV1 | null;
  onUpdateSettings: (updates: Partial<Settings>) => void;
  onBusyChange: (busy: boolean) => void;
}) {
  const [deviceId, setDeviceId] = useState(settings.microphone);
  const [promptIndex, setPromptIndex] = useState(0);
  const [phase, setPhase] = useState<RecorderPhase>('idle');
  const [level, setLevel] = useState(0);
  const [elapsedMs, setElapsedMs] = useState(0);
  const [summary, setSummary] = useState<CorpusSummary | null>(null);
  const [lastRecording, setLastRecording] = useState<CorpusRecordingEntry | null>(null);
  const [error, setError] = useState<string | null>(null);
  const startedAt = useRef<number | null>(null);
  const phaseRef = useRef<RecorderPhase>('idle');

  const busy = phase !== 'idle' && phase !== 'error';
  useEffect(() => {
    phaseRef.current = phase;
    onBusyChange(busy);
  }, [busy, onBusyChange]);

  useEffect(() => {
    let disposed = false;
    getCorpusSummary()
      .then((nextSummary) => {
        if (disposed) return;
        setSummary(nextSummary);
        const completed = new Set(
          nextSummary.recordings
            .filter((recording) => recording.selected)
            .map((recording) => recording.promptId),
        );
        const firstIncomplete = PERSONAL_CORPUS_PROMPTS.findIndex((prompt) => !completed.has(prompt.id));
        if (firstIncomplete >= 0) setPromptIndex(firstIncomplete);
      })
      .catch(() => {
        if (!disposed) setError('Could not prepare the private corpus summary.');
      });
    return () => {
      disposed = true;
      if (phaseRef.current !== 'idle' && phaseRef.current !== 'error') {
        void cancelCorpusRecording().catch(() => {});
      }
    };
  }, []);

  useEffect(() => {
    let statusUnlisten: (() => void) | undefined;
    let levelUnlisten: (() => void) | undefined;
    let disposed = false;
    listen<CorpusStatusEvent>('corpus-recording-status', (event) => {
      if (disposed) return;
      setPhase(event.payload.state);
      if (event.payload.error) setError(event.payload.error);
      if (event.payload.state === 'idle' || event.payload.state === 'error') {
        startedAt.current = null;
        setLevel(0);
      }
    }).then((unlisten) => {
      if (disposed) unlisten();
      else statusUnlisten = unlisten;
    }).catch(() => {});
    listen<number>('audio-level', (event) => {
      if (!disposed && phaseRef.current === 'recording') setLevel(event.payload);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else levelUnlisten = unlisten;
    }).catch(() => {});
    return () => {
      disposed = true;
      statusUnlisten?.();
      levelUnlisten?.();
    };
  }, []);

  useEffect(() => {
    if (phase !== 'recording') return;
    if (startedAt.current === null) startedAt.current = performance.now();
    const update = () => {
      if (startedAt.current !== null) setElapsedMs(performance.now() - startedAt.current);
    };
    update();
    const timer = window.setInterval(update, 100);
    return () => window.clearInterval(timer);
  }, [phase]);

  const {
    inventoryAvailable,
    displayDevices,
    selectableDevices,
    deviceSelectable,
  } = corpusMicrophoneAvailability(audioInventory, deviceId);
  const deviceOptions = useMemo(() => audioDeviceSelectOptions(selectableDevices), [selectableDevices]);
  const currentPrompt = PERSONAL_CORPUS_PROMPTS[promptIndex];
  const selectedDeviceLabel = deviceId === 'system_default'
    ? 'System Default'
    : displayDevices.find((device) => device.id === deviceId)?.name ?? 'Unavailable microphone';
  const selectedRecordings = summary?.recordings.filter((recording) => recording.selected) ?? [];
  const completedPromptIds = useMemo(
    () => new Set(selectedRecordings.map((recording) => recording.promptId)),
    [selectedRecordings],
  );
  const completedCount = PERSONAL_CORPUS_PROMPTS.filter((prompt) => completedPromptIds.has(prompt.id)).length;
  const currentTakeCount = summary?.recordings.filter((recording) => recording.promptId === currentPrompt.id).length ?? 0;

  const blockedReason = status !== 'idle'
    ? 'Finish the current dictation first.'
    : benchmarkRunning
      ? 'Finish the benchmark first.'
      : fileTranscribing
        ? 'Finish the file transcription first.'
        : null;
  const canRecord = phase === 'idle' && blockedReason === null && deviceSelectable;

  const beginRecording = useCallback(async () => {
    if (!canRecord) return;
    setError(null);
    setLastRecording(null);
    setElapsedMs(0);
    setLevel(0);
    startedAt.current = null;
    setPhase('starting');
    try {
      await startCorpusRecording({
        promptIndex: promptIndex + 1,
        prompt: currentPrompt,
        deviceId,
        deviceLabel: selectedDeviceLabel,
      });
      startedAt.current = performance.now();
      setPhase('recording');
    } catch (reason) {
      setPhase('error');
      setError(String(reason));
    }
  }, [canRecord, currentPrompt, deviceId, promptIndex, selectedDeviceLabel]);

  const finishRecording = useCallback(async () => {
    setError(null);
    setPhase('saving');
    try {
      const response = await stopCorpusRecording();
      setLastRecording(response.recording);
      setSummary((current) => ({
        corpusDirectory: response.corpusDirectory,
        recordings: [
          ...(current?.recordings ?? []).map((recording) => recording.promptId === response.recording.promptId
            ? { ...recording, selected: false }
            : recording),
          response.recording,
        ],
      }));
      setPhase('idle');
      startedAt.current = null;
      setLevel(0);
    } catch (reason) {
      setPhase('error');
      setError(String(reason));
    }
  }, []);

  const cancelRecording = useCallback(async () => {
    setError(null);
    setPhase('recovering');
    try {
      await cancelCorpusRecording();
      setLastRecording(null);
    } catch (reason) {
      setPhase('error');
      setError(String(reason));
    }
  }, []);

  const moveToPrompt = (nextIndex: number) => {
    setPromptIndex(Math.min(PERSONAL_CORPUS_PROMPTS.length - 1, Math.max(0, nextIndex)));
    setLastRecording(null);
    setError(null);
    setElapsedMs(0);
  };

  const useAndContinue = () => {
    const nextIncomplete = PERSONAL_CORPUS_PROMPTS.findIndex(
      (prompt, index) => index > promptIndex && !completedPromptIds.has(prompt.id),
    );
    moveToPrompt(nextIncomplete >= 0 ? nextIncomplete : Math.min(promptIndex + 1, PERSONAL_CORPUS_PROMPTS.length - 1));
  };

  return (
    <section className="space-y-4 rounded-xl border border-primary/25 bg-surface-container-lowest p-4">
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h3 className="text-sm font-semibold text-on-surface">Personal Corpus Recorder</h3>
            <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-on-surface">Private · local</span>
          </div>
          <p className="mt-1 text-xs leading-relaxed text-on-surface-variant">
            Record a reusable benchmark for your voice. Capture uses Murmur&apos;s signed worker, but does not transcribe, paste, transform, or add history.
          </p>
        </div>
        <div className="shrink-0 text-right">
          <p className="text-xs font-semibold tabular-nums text-on-surface">{completedCount}/{PERSONAL_CORPUS_PROMPTS.length}</p>
          <p className="text-[10px] text-on-surface-variant">prompts complete</p>
        </div>
      </div>

      <div className="h-1.5 overflow-hidden rounded-full bg-surface-container-high">
        <div
          className="h-full rounded-full bg-primary transition-all duration-200"
          style={{ width: `${(completedCount / PERSONAL_CORPUS_PROMPTS.length) * 100}%` }}
        />
      </div>

      <div>
        <label htmlFor="corpus-microphone" className="mb-1.5 block text-xs font-medium text-on-surface">Recording microphone</label>
        <select
          id="corpus-microphone"
          value={deviceId}
          disabled={busy || !inventoryAvailable}
          onChange={(event) => {
            const microphone = event.target.value;
            setDeviceId(microphone);
            onUpdateSettings({ microphone });
          }}
          className="w-full rounded-lg border border-outline-variant bg-surface-container-low px-3 py-2 text-xs text-on-surface outline-none focus:border-primary disabled:opacity-50"
        >
          <option value="system_default">System Default</option>
          {deviceOptions.map((device) => <option key={device.value} value={device.value}>{device.label}</option>)}
        </select>
      </div>

      <div className="settings-card p-4">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <span className="text-[10px] font-semibold uppercase tracking-wide text-primary">Prompt {promptIndex + 1} of {PERSONAL_CORPUS_PROMPTS.length}</span>
            <span className="rounded-full bg-surface-container-high px-2 py-0.5 text-[10px] text-on-surface-variant">{currentPrompt.category}</span>
            {completedPromptIds.has(currentPrompt.id) && <span className="text-[10px] font-medium text-success">Recorded</span>}
          </div>
          <span className="text-[10px] text-on-surface-variant">Next take {currentTakeCount + 1}</span>
        </div>
        <p className="mt-3 text-base font-medium leading-relaxed text-on-surface">“{currentPrompt.reference}”</p>
        <p className="mt-2 text-xs text-on-surface-variant">{currentPrompt.direction}</p>
      </div>

      <div className="space-y-2">
        <div className="flex items-center justify-between text-[11px] text-on-surface-variant">
          <span>{phase === 'starting' ? 'Connecting microphone…' : phase === 'saving' ? 'Saving WAV and manifest…' : phase === 'recovering' ? 'Stopping microphone…' : phase === 'recording' ? 'Recording' : 'Input level'}</span>
          <span className="tabular-nums">{phase === 'recording' ? durationLabel(elapsedMs) : selectedDeviceLabel}</span>
        </div>
        <div className="h-2 overflow-hidden rounded-full bg-surface-container-high">
          <div
            className={`h-full rounded-full transition-[width,background-color] duration-75 ${levelPercent(level) > 92 ? 'bg-error' : levelPercent(level) > 12 ? 'bg-success' : 'bg-on-surface-variant/40'}`}
            style={{ width: `${levelPercent(level)}%` }}
          />
        </div>
      </div>

      <div className="flex items-center justify-center gap-3">
        {phase === 'recording' ? (
          <button
            type="button"
            onClick={() => void finishRecording()}
            className="min-w-40 rounded-full border border-error/40 bg-error/10 px-6 py-3 text-sm font-semibold text-error shadow-sm hover:bg-error/15"
          >
            Stop & Save
          </button>
        ) : busy ? (
          <button
            type="button"
            onClick={() => void cancelRecording()}
            disabled={phase === 'saving' || phase === 'recovering'}
            className="min-w-40 rounded-full border border-outline-variant/40 px-6 py-3 text-sm font-semibold text-on-surface disabled:opacity-50"
          >
            {phase === 'saving' ? 'Saving…' : phase === 'recovering' ? 'Stopping…' : 'Cancel'}
          </button>
        ) : (
          <button
            type="button"
            onClick={() => void beginRecording()}
            disabled={!canRecord}
            className="min-w-40 rounded-full bg-primary px-6 py-3 text-sm font-semibold text-on-primary shadow-sm hover:bg-primary-dim disabled:cursor-not-allowed disabled:opacity-40"
          >
            {lastRecording ? 'Record Another Take' : 'Record'}
          </button>
        )}
      </div>

      {lastRecording && phase === 'idle' && (
        <div className={`rounded-lg border p-3 text-xs ${lastRecording.qualityWarnings.length > 0 ? 'border-primary/35 bg-surface-container-high' : 'border-success/30 bg-success/10'}`}>
          <div className="flex items-center justify-between gap-3">
            <p className="font-semibold text-on-surface">
              {lastRecording.qualityWarnings.length > 0 ? 'Saved with a quality note' : 'Saved and ready'}
            </p>
            <p className="tabular-nums text-on-surface-variant">Take {lastRecording.take} · {durationLabel(lastRecording.durationMs)}</p>
          </div>
          {lastRecording.qualityWarnings.map((warning) => <p key={warning} className="mt-1 text-primary">{warning}</p>)}
          <div className="mt-3 flex flex-wrap items-center gap-3">
            <button type="button" onClick={useAndContinue} className="rounded-(--ui-radius-pill) bg-primary shadow-(--ui-shadow-accent) px-3 py-1.5 font-semibold text-on-primary hover:bg-primary-dim">Use & Next</button>
            <button type="button" disabled={!canRecord} onClick={() => void beginRecording()} className="font-medium text-on-surface-variant underline hover:text-primary disabled:cursor-not-allowed disabled:opacity-40">Record another take</button>
          </div>
        </div>
      )}

      {blockedReason && <p className="text-xs text-primary">{blockedReason}</p>}
      {!inventoryAvailable && !error && <p className="text-xs text-primary">The microphone list is temporarily unavailable. Cached names are display-only until it refreshes.</p>}
      {inventoryAvailable && selectableDevices.length === 0 && !error && <p className="text-xs text-primary">No microphone input is currently available.</p>}
      {inventoryAvailable && deviceId !== 'system_default' && !deviceSelectable && !error && <p className="text-xs text-primary">The selected microphone is unavailable. Choose another microphone before recording.</p>}
      {error && (
        <div className="rounded-lg border border-error/35 bg-error/10 p-3 text-xs text-error">
          <p>{error}</p>
          {phase === 'error' && <button type="button" onClick={() => { setPhase('idle'); setError(null); }} className="mt-2 font-semibold underline">Reset recorder</button>}
        </div>
      )}

      <div className="flex items-center justify-between gap-3 border-t border-outline-variant/20 pt-3">
        <div className="flex gap-2">
          <button type="button" disabled={busy || promptIndex === 0} onClick={() => moveToPrompt(promptIndex - 1)} className="rounded-md border border-outline-variant/30 px-2.5 py-1.5 text-xs text-on-surface disabled:opacity-40">Previous</button>
          <button type="button" disabled={busy || promptIndex === PERSONAL_CORPUS_PROMPTS.length - 1} onClick={() => moveToPrompt(promptIndex + 1)} className="rounded-md border border-outline-variant/30 px-2.5 py-1.5 text-xs text-on-surface disabled:opacity-40">Next</button>
        </div>
        <button type="button" onClick={() => void openCorpusFolder().catch((reason) => setError(String(reason)))} className="text-xs font-medium text-on-surface-variant underline hover:text-primary">Reveal recordings</button>
      </div>

      <p className="break-all text-[10px] leading-relaxed text-on-surface-variant">
        {summary?.corpusDirectory ?? 'Preparing private corpus folder…'} · WAV files and reference text remain on this Mac and are not added to Git.
      </p>
    </section>
  );
}
