import { noDefaultButtonKeyDown } from '../../lib/defaultButton';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { modelStatusLabel, type ModelRuntimeSnapshot } from '../../lib/modelRuntime';
import { Button } from '../ui/FormControls';

export function TranscriptionModelStorage({ models, selectedModel, busy }: {
  models: ModelRuntimeSnapshot[];
  selectedModel: string;
  busy: boolean;
}) {
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setConfirmRemove(null), [selectedModel, busy]);

  async function run(modelName: string, remove: boolean) {
    if (pendingRef.current || busy) return;
    if (remove && confirmRemove !== modelName) {
      setConfirmRemove(modelName);
      return;
    }
    pendingRef.current = true;
    setPending(modelName);
    setConfirmRemove(null);
    setError(null);
    try {
      await invoke(remove ? 'remove_model' : 'download_model', { modelName });
    } catch (reason: unknown) {
      setError(String(reason));
    } finally {
      pendingRef.current = false;
      setPending(null);
    }
  }

  return (
    <div className="settings-field" aria-label="Transcription model storage">
      <p className="text-[length:var(--ui-font-label)] font-medium text-on-surface">Model storage</p>
      <p className="text-[length:var(--ui-font-caption)] text-on-surface-variant">Downloaded models take up disk space. Remove the ones you no longer use.</p>
      {models.map((model) => {
        const installed = model.installState === 'installed';
        const active = model.modelName === selectedModel
          || !['unloaded', 'failed'].includes(model.lifecycleState);
        const installing = ['installing', 'validating'].includes(model.installState);
        const selected = model.modelName === selectedModel;
        const canRemove = installed && !active;
        const canDownload = !installed && !installing && model.supported && !selected;
        const status = modelStatusLabel(model);
        return (
          <div key={model.modelName} data-testid="model-storage-row" className="flex items-center gap-3 rounded-lg border border-outline-variant/30 px-3 py-2">
            <span className="min-w-0 flex-1 text-[length:var(--ui-font-caption)] text-on-surface">
              <span className="block truncate font-medium">{model.label}</span>
              <span className="text-on-surface-variant">{model.size} · {status}{!selected && active ? ' · In use' : ''}</span>
            </span>
            {selected && (
              <span className="shrink-0 rounded-full bg-[var(--ui-tint-accent-strong)] px-2 py-0.5 text-[length:var(--ui-font-caption)] font-semibold text-on-surface">Selected</span>
            )}
            {canRemove && (
              <Button type="button" variant="ghost" size="xs" disabled={busy || pending !== null}
                onKeyDown={noDefaultButtonKeyDown} onClick={() => void run(model.modelName, true)}
                onBlur={() => setConfirmRemove(null)}
                aria-label={`${confirmRemove === model.modelName ? 'Confirm remove' : 'Remove'} ${model.label}…`}>
                {pending === model.modelName ? 'Working…' : confirmRemove === model.modelName ? 'Confirm remove…' : 'Remove…'}
              </Button>
            )}
            {canDownload && (
              <Button type="button" variant="ghost" size="xs" disabled={busy || pending !== null}
                aria-label={`Download ${model.label}`} onClick={() => void run(model.modelName, false)}>
                {pending === model.modelName ? 'Working…' : 'Download'}
              </Button>
            )}
          </div>
        );
      })}
      {error && <p role="alert" className="text-[length:var(--ui-font-caption)] text-error">{error}</p>}
    </div>
  );
}
