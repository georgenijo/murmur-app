import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ModelRuntimeSnapshot } from '../../lib/modelRuntime';

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
      <p className="text-sm font-medium text-on-surface">Model storage</p>
      <p className="text-xs text-on-surface-variant">Remove unused models to free storage. Choose another model before removing the selected one.</p>
      {models.map((model) => {
        const installed = model.installState === 'installed';
        const active = model.modelName === selectedModel
          || !['unloaded', 'failed'].includes(model.lifecycleState);
        const installing = ['installing', 'validating'].includes(model.installState);
        return (
          <div key={model.modelName} className="flex items-center gap-3 rounded-lg border border-outline-variant/30 px-3 py-2">
            <span className="min-w-0 flex-1 text-xs text-on-surface">
              <span className="block font-medium">{model.label}</span>
              <span className="text-on-surface-variant">{model.size} · {installed ? 'Installed' : installing ? 'Installing…' : 'Not installed'}{model.modelName === selectedModel ? ' · Selected' : active ? ' · In use' : ''}</span>
            </span>
            {installed && !active && (
              <button type="button" disabled={busy || pending !== null}
                onClick={() => void run(model.modelName, true)}
                onBlur={() => setConfirmRemove(null)}
                aria-label={`${confirmRemove === model.modelName ? 'Confirm remove' : 'Remove'} ${model.label}`}
                className="settings-quiet-btn px-3 py-1.5 text-xs font-medium text-on-surface-variant disabled:opacity-50">
                {pending === model.modelName ? 'Working…' : confirmRemove === model.modelName ? 'Confirm remove' : 'Remove'}
              </button>
            )}
            {!installed && !installing && model.supported && model.modelName !== selectedModel && (
              <button type="button" disabled={busy || pending !== null}
                aria-label={`Download ${model.label}`} onClick={() => void run(model.modelName, false)}
                className="settings-quiet-btn px-3 py-1.5 text-xs font-medium text-primary disabled:opacity-50">
                {pending === model.modelName ? 'Working…' : 'Download'}
              </button>
            )}
          </div>
        );
      })}
      {error && <p role="alert" className="text-xs text-error">{error}</p>}
    </div>
  );
}
