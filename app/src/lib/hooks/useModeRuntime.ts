import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type ModeSource = 'manual' | 'app_binding' | 'site_binding' | 'temporary';

export interface ModeRuntimeStatus {
  id: string;
  name: string;
  source: ModeSource;
}

const FALLBACK: ModeRuntimeStatus = { id: 'builtin.everyday', name: 'Everyday', source: 'manual' };

function validStatus(value: unknown): value is ModeRuntimeStatus {
  const status = value as Partial<ModeRuntimeStatus> | null;
  return typeof status?.id === 'string' && status.id.length > 0 && status.id.length <= 128
    && typeof status.name === 'string' && status.name.length > 0 && status.name.length <= 128
    && (status.source === 'manual' || status.source === 'app_binding'
      || status.source === 'site_binding' || status.source === 'temporary');
}

export function useModeRuntime() {
  const [status, setStatus] = useState<ModeRuntimeStatus>(FALLBACK);

  useEffect(() => {
    void invoke<unknown>('get_mode_runtime_status').then((value) => {
      if (validStatus(value)) setStatus(value);
    }).catch(() => {});
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void listen<unknown>('mode-runtime-changed', ({ payload }) => {
      if (validStatus(payload)) setStatus(payload);
    }).then((fn) => {
      if (cancelled) fn(); else unlisten = fn;
    }).catch(() => {});
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  const cycle = useCallback(async () => {
    const next = await invoke<unknown>('cycle_mode');
    if (validStatus(next)) setStatus(next);
  }, []);

  const clearTemporary = useCallback(async () => {
    const next = await invoke<unknown>('clear_temporary_mode_override');
    if (validStatus(next)) setStatus(next);
  }, []);

  return { status, cycle, clearTemporary };
}
