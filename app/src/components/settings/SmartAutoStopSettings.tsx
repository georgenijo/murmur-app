import { DEFAULT_SETTINGS } from '../../lib/settings';
import { SettingsRow } from './SettingsRow';
import { useEffect, useState } from 'react';
import {
  downloadSmartTurnModel,
  removeSmartTurnModel,
  SMART_TURN_MODEL_SIZE_LABEL,
  smartTurnModelStatus,
  type SmartTurnModelStatus,
} from '../../lib/smartTurn';
import AnimatedSwitch from '../ui/animated-switch/animated-switch';

interface SmartAutoStopSettingsProps {
  enabled: boolean;
  /** Stop on Silence is the ceiling; without it there is nothing to shorten. */
  silenceAutoStopOn: boolean;
  /** Locked mid-recording like Stop on Silence: the hook reads it live. */
  isRecording: boolean;
  onEnabledChange: (enabled: boolean) => void;
}

/** Smart Auto-Stop toggle plus its user-initiated Smart Turn model row. */
export function SmartAutoStopSettings({
  enabled,
  silenceAutoStopOn,
  isRecording,
  onEnabledChange,
}: SmartAutoStopSettingsProps) {
  const [status, setStatus] = useState<SmartTurnModelStatus | null>(null);
  const [operation, setOperation] = useState<'idle' | 'downloading' | 'removing'>('idle');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    void smartTurnModelStatus()
      .then((next) => { if (!disposed) setStatus(next); })
      .catch((cause) => { if (!disposed) setError(String(cause)); });
    return () => { disposed = true; };
  }, []);

  // A download outlives this settings page. Refresh a remounted page until
  // the backend finishes, including failures that allow a retry.
  useEffect(() => {
    if (status?.state !== 'downloading') return;
    let disposed = false;
    const timer = window.setInterval(() => {
      void smartTurnModelStatus()
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
      setStatus(await smartTurnModelStatus());
    } catch (cause) {
      setError(String(cause));
    } finally {
      setOperation('idle');
    }
  };

  const ready = status?.state === 'ready';
  const busy = operation !== 'idle' || status?.state === 'downloading';

  return (
    <div data-setting-target="smart-auto-stop" className="settings-stack rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
      <SettingsRow title="Smart Auto-Stop (Experimental)" description="At each pause, a small local model listens to how you stopped speaking and finishes the recording early when you sound done. Stop on Silence stays the longest it will wait, so this needs Stop on Silence on. Without the model it changes nothing." onReset={enabled !== DEFAULT_SETTINGS.smartAutoStopEnabled ? () => onEnabledChange(DEFAULT_SETTINGS.smartAutoStopEnabled) : undefined} disabled={isRecording || (!enabled && !silenceAutoStopOn)} control={<AnimatedSwitch
          size="md"
          checked={enabled}
          disabled={isRecording || (!enabled && !silenceAutoStopOn)}
          aria-label="Smart Auto-Stop"
          onCheckedChange={() => onEnabledChange(!enabled)}
        />} />

      <div className="flex flex-wrap items-center justify-between gap-3 rounded-[var(--ui-radius-control)] border border-outline-variant/30 px-3 py-2.5">
        <div>
          <p className="text-xs font-semibold text-on-surface">End-of-turn model</p>
          <p className="mt-0.5 text-[11px] text-on-surface-variant">
            {!status
              ? 'Checking this Mac...'
              : ready
                ? `Installed locally, ${SMART_TURN_MODEL_SIZE_LABEL}`
                : `Optional download, ${SMART_TURN_MODEL_SIZE_LABEL}`}
          </p>
          <p className="mt-1 text-[10px] text-on-surface-variant">
            Smart Turn v3.2 by <a href="https://huggingface.co/pipecat-ai/smart-turn-v3" target="_blank" rel="noreferrer" className="underline hover:text-primary">Pipecat</a>, licensed <a href="https://opensource.org/license/bsd-2-clause" target="_blank" rel="noreferrer" className="underline hover:text-primary">BSD 2-Clause</a>. Audio never leaves this Mac.
          </p>
        </div>
        {ready ? (
          <button type="button" disabled={busy || isRecording} onClick={() => void run('removing', removeSmartTurnModel)} className="rounded-[var(--ui-radius-control)] px-3 py-1.5 text-xs font-semibold text-error disabled:opacity-40">
            {operation === 'removing' ? 'Removing...' : 'Remove model'}
          </button>
        ) : (
          <button type="button" disabled={busy || !status} onClick={() => void run('downloading', downloadSmartTurnModel)} className="dialog-pill-btn px-3 py-1.5 text-xs font-semibold text-primary disabled:opacity-40">
            {busy ? 'Downloading...' : 'Download model'}
          </button>
        )}
      </div>
      {error && <p role="alert" className="text-xs text-error">{error}</p>}
    </div>
  );
}
