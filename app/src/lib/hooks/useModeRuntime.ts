import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type ModeSource = 'manual' | 'app_binding' | 'site_binding' | 'temporary';

export interface ModeChoice { id: string; name: string }

export interface ModeRuntimeStatus {
  id: string;
  name: string;
  source: ModeSource;
  pending?: ModeChoice | null;
  pendingRevision?: number;
  available?: ModeChoice[];
}

const FALLBACK: ModeRuntimeStatus = { id: 'builtin.everyday', name: 'Everyday', source: 'manual' };

function validStatus(value: unknown): value is ModeRuntimeStatus {
  const status = value as Partial<ModeRuntimeStatus> | null;
  return typeof status?.id === 'string' && status.id.length > 0 && status.id.length <= 128
    && typeof status.name === 'string' && status.name.length > 0 && status.name.length <= 128
    && (status.source === 'manual' || status.source === 'app_binding'
      || status.source === 'site_binding' || status.source === 'temporary')
    && (status.pending == null || validChoice(status.pending))
    && (status.pendingRevision === undefined || (Number.isSafeInteger(status.pendingRevision) && status.pendingRevision >= 0))
    && (status.available === undefined || (Array.isArray(status.available)
      && status.available.length <= 107 && status.available.every(validChoice)));
}

function validChoice(value: unknown): value is ModeChoice {
  const choice = value as Partial<ModeChoice> | null;
  return typeof choice?.id === 'string' && choice.id.length > 0 && choice.id.length <= 128
    && typeof choice.name === 'string' && choice.name.length > 0 && choice.name.length <= 128;
}

export function useModeRuntime() {
  const [status, setStatus] = useState<ModeRuntimeStatus>(FALLBACK);
  const acceptStatus = useCallback((value: unknown) => {
    if (validStatus(value)) setStatus((previous) =>
      (value.pendingRevision ?? 0) >= (previous.pendingRevision ?? 0) ? value : previous);
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void listen<unknown>('mode-runtime-changed', ({ payload }) => {
      if (!cancelled) acceptStatus(payload);
    }).then(async (fn) => {
      if (cancelled) { fn(); return; }
      unlisten = fn;
      // Subscribe before reading, so an arm/consume between setup steps cannot
      // be missed. Revisions prevent a delayed response restoring old intent.
      const value = await invoke<unknown>('get_mode_runtime_status');
      if (!cancelled) acceptStatus(value);
    }).catch(() => {});
    return () => { cancelled = true; unlisten?.(); };
  }, [acceptStatus]);

  const cycle = useCallback(async () => {
    const next = await invoke<unknown>('cycle_mode');
    acceptStatus(next);
  }, [acceptStatus]);

  const clearTemporary = useCallback(async () => {
    const next = await invoke<unknown>('clear_temporary_mode_override');
    acceptStatus(next);
  }, [acceptStatus]);

  const setNext = useCallback(async (modeId: string | null) => {
    const next = await invoke<unknown>('set_next_recording_mode', { modeId });
    acceptStatus(next);
  }, [acceptStatus]);

  return { status, cycle, clearTemporary, setNext };
}
