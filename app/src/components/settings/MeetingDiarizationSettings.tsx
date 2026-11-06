import { DEFAULT_SETTINGS } from '../../lib/settings';
import { SettingsRow } from './SettingsRow';
import { ConfirmActionButton } from '../ui/ConfirmActionButton';
import { useEffect, useState } from 'react';
import { listen } from '../../lib/targetedEvents';
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
import { SettingsBranch } from './SettingsBranch';

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

  if (!supported) return null;

  return (
    <div data-setting-target="meeting-speakers" className="settings-stack rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
      <SettingsRow title="Label remote speakers" description="Label other speakers after a meeting" onReset={enabled !== DEFAULT_SETTINGS.meetingDiarization ? () => onEnabledChange(DEFAULT_SETTINGS.meetingDiarization) : undefined} disabled={!supported} control={<AnimatedSwitch
          size="md"
          checked={enabled}
          disabled={!supported}
          aria-label="Label remote speakers"
          onCheckedChange={() => onEnabledChange(!enabled)}
        />} />

      <SettingsBranch open={enabled} className="settings-branch-stack">
      <div className="settings-callout text-[length:var(--ui-font-caption)] leading-relaxed text-on-surface-variant">
        <p>
          Labels stay on this Mac and apply only within one meeting. Murmur temporarily saves up to two hours of system audio for labeling, then deletes the temporary copy. Keep Meeting Audio is a separate setting.
        </p>
        <p className="mt-2">
          If a meeting exceeds two hours, speaker labeling is skipped while capture and transcription continue. Uncertain passages remain Them. Murmur does not create voice profiles across meetings.
        </p>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3 rounded-[var(--ui-radius-control)] border border-outline-variant/30 px-3 py-2.5">
        <div>
          <p className="text-[length:var(--ui-font-caption)] font-semibold text-on-surface">Remote speaker model</p>
          <p className="mt-0.5 text-[length:var(--ui-font-caption)] text-on-surface-variant">
            {!status
              ? 'Checking this Mac…'
              : !status.supported
                ? 'Requires an Apple Silicon Mac.'
                : status.installed
                  ? `Installed locally, ${modelSize(status.bytes)}`
                  : `Optional download, ${modelSize(status.bytes)}`}
          </p>
          <p className="mt-1 text-[length:var(--ui-font-caption)] text-on-surface-variant">
            Model by <a href="https://huggingface.co/FluidInference/speaker-diarization-coreml" target="_blank" rel="noreferrer" className="underline hover:text-primary">FluidInference</a>, licensed <a href="https://creativecommons.org/licenses/by/4.0/" target="_blank" rel="noreferrer" className="underline hover:text-primary">CC BY 4.0</a>.
          </p>
        </div>
        {status?.installed ? (
          <ConfirmActionButton label="Remove model" confirmLabel="Confirm remove" disabled={busy} onConfirm={() => void remove()} className="rounded-[var(--ui-radius-control)] px-3 py-1.5 text-[length:var(--ui-font-caption)] font-semibold text-error disabled:opacity-40" />
        ) : (
          <button type="button" disabled={busy || !supported || !status} onClick={() => void download()} className="dialog-pill-btn px-3 py-1.5 text-[length:var(--ui-font-caption)] font-semibold text-primary disabled:opacity-40">
            {busy ? 'Installing…' : 'Download model'}
          </button>
        )}
      </div>
      {operation === 'downloading' && progress && (
        <div role="status" className="text-[length:var(--ui-font-caption)] text-on-surface-variant">
          {modelDownloadLabel(progress)}{percentage === null ? '' : ` ${percentage}%`}
        </div>
      )}
      {error && <p role="alert" className="text-[length:var(--ui-font-caption)] text-error">{error}</p>}
      </SettingsBranch>
    </div>
  );
}
