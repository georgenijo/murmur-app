import { DEFAULT_SETTINGS } from '../../lib/settings';
import { SettingsRow } from './SettingsRow';
import { ConfirmActionButton } from '../ui/ConfirmActionButton';
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
import { SettingsBranch } from './SettingsBranch';
import { SettingsDisclosure } from './SettingsLayout';

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
  const [howItWorksOpen, setHowItWorksOpen] = useState(false);

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
    <>
      <SettingsRow targetId="meeting-speakers" title="Label remote speakers" compact onReset={enabled !== DEFAULT_SETTINGS.meetingDiarization ? () => onEnabledChange(DEFAULT_SETTINGS.meetingDiarization) : undefined} disabled={!supported} control={<AnimatedSwitch
          size="md"
          checked={enabled}
          disabled={!supported}
          aria-label="Label remote speakers"
          onCheckedChange={() => onEnabledChange(!enabled)}
        />} />

      <SettingsBranch open={enabled} className="settings-branch-stack">
      <div className="settings-rows">
      <SettingsDisclosure title="How speaker labeling works" layout="stack" open={howItWorksOpen} onOpenChange={setHowItWorksOpen}>
        <p className="text-[length:var(--ui-font-caption)] leading-relaxed text-on-surface-variant">
          Labels stay on this Mac and apply only within this meeting. Murmur temporarily saves system audio for labeling, up to two hours, then deletes it; Keep meeting audio is separate. Over two hours, labeling is skipped while capture and transcription continue. Uncertain passages stay Them. No cross-meeting voice profiles.
        </p>
      </SettingsDisclosure>
      <SettingsRow title="Remote speaker model" description={
        !status
          ? 'Checking this Mac…'
          : !status.supported
            ? 'Requires an Apple Silicon Mac.'
            : <>
                {status.installed ? 'Local' : 'Optional'} · {modelSize(status.bytes)} · <a href="https://huggingface.co/FluidInference/speaker-diarization-coreml" target="_blank" rel="noreferrer" className="underline hover:text-primary">FluidInference</a> · <a href="https://creativecommons.org/licenses/by/4.0/" target="_blank" rel="noreferrer" className="underline hover:text-primary">CC BY 4.0</a>
              </>
      } control={status?.installed ? (
        <ConfirmActionButton label="Remove model" confirmLabel="Confirm remove" disabled={busy} onConfirm={() => void remove()} className="ui-form-button text-error" />
      ) : (
        <button type="button" disabled={busy || !supported || !status} onClick={() => void download()} className="ui-form-button text-primary">
          {busy ? 'Installing…' : 'Download model'}
        </button>
      )} />
      {operation === 'downloading' && progress && (
        <div role="status" className="text-[length:var(--ui-font-caption)] text-on-surface-variant">
          {modelDownloadLabel(progress)}{percentage === null ? '' : ` ${percentage}%`}
        </div>
      )}
      {error && <p role="alert" className="text-[length:var(--ui-font-caption)] text-error">{error}</p>}
      </div>
      </SettingsBranch>
    </>
  );
}
