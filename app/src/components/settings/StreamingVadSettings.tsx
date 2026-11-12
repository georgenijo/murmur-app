import { useEffect, useRef, useState } from 'react';
import { DEFAULT_SETTINGS } from '../../lib/settings';
import {
  downloadStreamingVadModel,
  removeStreamingVadModel,
  STREAMING_VAD_MODEL_SIZE_LABEL,
  streamingVadModelStatus,
  type StreamingVadModelStatus,
} from '../../lib/streamingVad';
import { ConfirmActionButton } from '../ui/ConfirmActionButton';
import { SettingToggle } from './SettingToggle';
import { SettingsBranch } from './SettingsBranch';

interface StreamingVadSettingsProps {
  enabled: boolean;
  isRecording: boolean;
  onEnabledChange: (enabled: boolean) => void;
}

/**
 * Streaming speech detection toggle plus its user-initiated Silero model row.
 * Switching the setting on is the user's request to download the model; a
 * recording never downloads it, and without the model the batch VAD runs.
 */
export function StreamingVadSettings({ enabled, isRecording, onEnabledChange }: StreamingVadSettingsProps) {
  const [status, setStatus] = useState<StreamingVadModelStatus | null>(null);
  const [operation, setOperation] = useState<'idle' | 'downloading' | 'removing'>('idle');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    void streamingVadModelStatus()
      .then((next) => { if (!disposed) setStatus(next); })
      .catch((cause) => { if (!disposed) setError(String(cause)); });
    return () => { disposed = true; };
  }, []);

  // A download outlives this settings page. Refresh a remounted page until the
  // backend finishes, including failures that allow a retry.
  useEffect(() => {
    if (status?.state !== 'downloading') return;
    let disposed = false;
    const timer = window.setInterval(() => {
      void streamingVadModelStatus()
        .then((next) => { if (!disposed) setStatus(next); })
        .catch((cause) => { if (!disposed) setError(String(cause)); });
    }, 1000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, [status?.state]);

  const run = async (kind: 'downloading' | 'removing', action: () => Promise<void>) => {
    setOperation(kind);
    setError(null);
    try {
      await action();
    } catch (cause) {
      setError(String(cause));
    }
    try {
      setStatus(await streamingVadModelStatus());
    } catch (cause) {
      setError(String(cause));
    } finally {
      setOperation('idle');
    }
  };

  const busy = operation !== 'idle' || status?.state === 'downloading';
  const ready = status?.state === 'ready';
  const failed = status?.state === 'failed';

  // Switching on before the first status arrives is still the user's request:
  // remember it and act once the status resolves.
  const wantsModel = useRef(false);
  useEffect(() => {
    if (!enabled) wantsModel.current = false;
  }, [enabled]);
  useEffect(() => {
    if (!wantsModel.current || !status) return;
    wantsModel.current = false;
    if (enabled && status.state !== 'ready' && status.state !== 'downloading' && operation === 'idle') {
      void run('downloading', downloadStreamingVadModel);
    }
    // `run` only touches state setters and module imports.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [status]);

  const toggle = () => {
    const next = !enabled;
    onEnabledChange(next);
    // Turning it on is the user's request for the model.
    if (!next) return;
    if (!status) {
      wantsModel.current = true;
    } else if (status.state !== 'ready' && !busy) {
      void run('downloading', downloadStreamingVadModel);
    }
  };

  let summary = 'Checking this Mac…';
  if (status) {
    if (ready) summary = `Installed locally, ${STREAMING_VAD_MODEL_SIZE_LABEL}`;
    else if (busy) summary = 'Downloading…';
    else if (failed) summary = 'Download or verification failed';
    else summary = `Not downloaded, ${STREAMING_VAD_MODEL_SIZE_LABEL}`;
  }

  return (
    <div className="settings-stack">
      <SettingToggle
        targetId="streaming-pipeline"
        title="Streaming speech detection (experimental)"
        description="Find speech while you record so stopping is faster on long dictations. Needs a small model; until it is installed, recordings use the standard detection. Applies to the next recording."
        checked={enabled}
        defaultChecked={DEFAULT_SETTINGS.streamingPipelineEnabled}
        onChange={toggle}
      />
      <SettingsBranch open={enabled} className="settings-branch-stack">
        <div data-streaming-vad-model className="settings-dependent-card flex flex-wrap items-center justify-between gap-3 rounded-[var(--ui-radius-control)] border border-outline-variant/30 px-3 py-2.5">
          <div>
            <p className="text-[length:var(--ui-font-caption)] font-semibold text-on-surface">Speech detection model</p>
            <p className="mt-0.5 text-[length:var(--ui-font-caption)] text-on-surface-variant">{summary}</p>
            <p className="mt-1 text-[length:var(--ui-font-caption)] text-on-surface-variant">
              Silero VAD v5.1.2 by <a href="https://github.com/snakers4/silero-vad" target="_blank" rel="noreferrer" className="underline hover:text-primary">Silero Team</a>, licensed <a href="https://opensource.org/license/mit" target="_blank" rel="noreferrer" className="underline hover:text-primary">MIT</a>. Audio never leaves this Mac.
            </p>
          </div>
          {ready ? (
            <ConfirmActionButton label="Remove model" confirmLabel="Confirm remove" disabled={busy || isRecording} onConfirm={() => void run('removing', removeStreamingVadModel)} className="rounded-[var(--ui-radius-control)] px-3 py-1.5 text-[length:var(--ui-font-caption)] font-semibold text-error disabled:opacity-40" />
          ) : (
            <button type="button" disabled={busy || !status} onClick={() => void run('downloading', downloadStreamingVadModel)} className="dialog-pill-btn px-3 py-1.5 text-[length:var(--ui-font-caption)] font-semibold text-primary disabled:opacity-40">
              {busy ? 'Downloading…' : failed ? 'Retry download' : 'Download model'}
            </button>
          )}
        </div>
        {(error ?? status?.error) && <p role="alert" className="text-[length:var(--ui-font-caption)] text-error">{error ?? status?.error}</p>}
      </SettingsBranch>
    </div>
  );
}
