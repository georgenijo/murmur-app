import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  downloadDiarizationModel,
  getDiarizationModelStatus,
  MEETING_DIARIZATION_MODEL_ID,
  removeDiarizationModel,
  type DiarizationModelStatus,
} from '../../lib/meetings';
import {
  modelDownloadLabel,
  modelDownloadPercent,
  type ModelDownloadProgress,
} from '../../lib/modelDownload';
import AnimatedSwitch from '../ui/animated-switch/animated-switch';

interface MeetingDiarizationSettingsProps {
  enabled: boolean;
  onEnabledChange: (enabled: boolean) => void;
}

function modelSize(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function MeetingDiarizationSettings({
  enabled,
  onEnabledChange,
}: MeetingDiarizationSettingsProps) {
  const [status, setStatus] = useState<DiarizationModelStatus | null>(null);
  const [operation, setOperation] = useState<'idle' | 'downloading' | 'removing'>('idle');
  const [progress, setProgress] = useState<ModelDownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refreshStatus = async () => {
    setStatus(await getDiarizationModelStatus());
  };

  useEffect(() => {
    let disposed = false;
    void getDiarizationModelStatus()
      .then((next) => {
        if (!disposed) setStatus(next);
      })
      .catch((cause) => {
        if (!disposed) setError(String(cause));
      });
    return () => {
      disposed = true;
    };
  }, []);

  useEffect(() => {
    if (status?.installing !== true) return;
    let disposed = false;
    const timer = window.setInterval(() => {
      void getDiarizationModelStatus()
        .then((next) => {
          if (!disposed) setStatus(next);
        })
        .catch((cause) => {
          if (!disposed) setError(String(cause));
        });
    }, 1_000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [status?.installing]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<ModelDownloadProgress>('download-progress', (event) => {
      if (event.payload.modelName === MEETING_DIARIZATION_MODEL_ID) {
        setProgress(event.payload);
      }
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const download = async () => {
    setOperation('downloading');
    setProgress(null);
    setError(null);
    try {
      await downloadDiarizationModel();
      await refreshStatus();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setOperation('idle');
      setProgress(null);
    }
  };

  const remove = async () => {
    setOperation('removing');
    setError(null);
    try {
      await removeDiarizationModel();
      await refreshStatus();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setOperation('idle');
    }
  };

  const percentage = progress ? modelDownloadPercent(progress) : null;
  const busy = operation !== 'idle' || status?.installing === true;
  const supported = status?.supported !== false;

  return (
    <div data-setting-target="meeting-speakers" className="settings-setting-group space-y-3 rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
      <div className="settings-setting-row flex items-center justify-between gap-6 px-1">
        <div>
          <p className="text-sm font-medium text-on-surface">Label Remote Speakers</p>
          <p className="mt-0.5 text-xs leading-relaxed text-on-surface-variant">
            Add local Speaker 1, Speaker 2 labels to clear system-audio passages after a meeting.
          </p>
        </div>
        <AnimatedSwitch
          size="md"
          checked={enabled}
          disabled={!supported}
          aria-label="Label Remote Speakers"
          onCheckedChange={() => onEnabledChange(!enabled)}
        />
      </div>

      <div className="rounded-[var(--ui-radius-control)] border border-outline-variant/30 bg-surface-container-lowest p-3 text-xs leading-relaxed text-on-surface-variant">
        <p>
          Labels stay on this Mac and apply only within one meeting. Murmur temporarily saves up to two hours of system audio for labeling, then deletes the temporary copy. Keep Meeting Audio is a separate setting.
        </p>
        <p className="mt-2">
          If a meeting exceeds two hours, speaker labeling is skipped while capture and transcription continue. Uncertain passages remain Them. Murmur does not create voice profiles across meetings.
        </p>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3 rounded-[var(--ui-radius-control)] border border-outline-variant/30 px-3 py-2.5">
        <div>
          <p className="text-xs font-semibold text-on-surface">Remote speaker model</p>
          <p className="mt-0.5 text-[11px] text-on-surface-variant">
            {!status
              ? 'Checking this Mac...'
              : !status.supported
                ? 'Requires an Apple Silicon Mac.'
                : status.installed
                  ? `Installed locally, ${modelSize(status.bytes)}`
                  : `Optional download, ${modelSize(status.bytes)}`}
          </p>
          <p className="mt-1 text-[10px] text-on-surface-variant">
            Model by <a href="https://huggingface.co/FluidInference/speaker-diarization-coreml" target="_blank" rel="noreferrer" className="underline hover:text-primary">FluidInference</a>, licensed <a href="https://creativecommons.org/licenses/by/4.0/" target="_blank" rel="noreferrer" className="underline hover:text-primary">CC BY 4.0</a>.
          </p>
        </div>
        {status?.installed ? (
          <button type="button" disabled={busy} onClick={() => void remove()} className="rounded-[var(--ui-radius-control)] px-3 py-1.5 text-xs font-semibold text-error disabled:opacity-40">
            {operation === 'removing' ? 'Removing...' : 'Remove model'}
          </button>
        ) : (
          <button type="button" disabled={busy || !supported || !status} onClick={() => void download()} className="dialog-pill-btn px-3 py-1.5 text-xs font-semibold text-primary disabled:opacity-40">
            {busy ? 'Installing...' : 'Download model'}
          </button>
        )}
      </div>
      {operation === 'downloading' && progress && (
        <div role="status" className="text-xs text-on-surface-variant">
          {modelDownloadLabel(progress)}{percentage === null ? '' : ` ${percentage}%`}
        </div>
      )}
      {error && <p role="alert" className="text-xs text-error">{error}</p>}
    </div>
  );
}
