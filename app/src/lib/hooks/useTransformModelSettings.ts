import { useCallback, useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { Settings, TransformKey } from '../settings';
import {
  downloadTransformModel,
  removeTransformModel,
  resetTransformRuntime,
  setTransformKey,
  startTransformListener,
  stopTransformListener,
  transformModelStatus,
  type TransformModelDownloadProgress,
  type TransformModelStatus,
} from '../transformSettings';

export interface UseTransformModelSettingsParams {
  settings: Settings;
  onUpdateSettings: (updates: Partial<Settings>) => void;
  /** True while the AI overview or Selected-Text Rewrite page is active —
   *  gates the status-refresh effect the same way `activeCat` used to. */
  active: boolean;
}

/**
 * State, effects, and handlers for the Selected-Text Rewrite (transform)
 * model block (F7), extracted from `SettingsPanel.tsx`. Behavior-preserving.
 */
export function useTransformModelSettings({ settings, onUpdateSettings, active }: UseTransformModelSettingsParams) {
  const [transformModel, setTransformModel] = useState<TransformModelStatus | null>(null);
  const [transformModelBusy, setTransformModelBusy] = useState(false);
  const [transformModelError, setTransformModelError] = useState<string | null>(null);
  const [confirmRemoveTransform, setConfirmRemoveTransform] = useState(false);
  // Shortcut-picker failures get their own error line, separate from the model
  // block's error slot (#312 D1 round-2 finding 8).
  const [transformKeyError, setTransformKeyError] = useState<string | null>(null);
  const [transformDownloadPct, setTransformDownloadPct] = useState<number | null>(null);

  const refreshTransformModel = useCallback(async () => {
    try {
      setTransformModel(await transformModelStatus());
    } catch {
      setTransformModel(null);
    }
  }, []);

  useEffect(() => {
    if (!active) return;
    void refreshTransformModel();
  }, [active, refreshTransformModel]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<TransformModelDownloadProgress>(
      'transform-model-download-progress',
      (event) => {
        const { received = 0, total = 0 } = event.payload;
        if (total > 0) setTransformDownloadPct(Math.min(100, Math.round((received / total) * 100)));
        else setTransformDownloadPct(null);
      },
    )
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    return () => {
      unlisten?.();
    };
  }, []);

  const updateTransformHoldKey = async (next: TransformKey | null) => {
    setTransformKeyError(null);
    if (next !== null && next === settings.queryHotkey) {
      setTransformKeyError('That key is already assigned to Voice Query.');
      return;
    }
    try {
      if (next === null) {
        await stopTransformListener();
        onUpdateSettings({ transformHoldKey: null });
        return;
      }
      await setTransformKey(next);
      await startTransformListener(next);
      onUpdateSettings({ transformHoldKey: next });
    } catch (e) {
      setTransformKeyError(String(e));
    }
  };

  const downloadTransform = async () => {
    setTransformModelBusy(true);
    setTransformModelError(null);
    setTransformDownloadPct(0);
    try {
      await downloadTransformModel();
      await refreshTransformModel();
    } catch (e) {
      setTransformModelError(String(e));
    } finally {
      setTransformModelBusy(false);
      setTransformDownloadPct(null);
    }
  };

  const removeTransform = async () => {
    if (!confirmRemoveTransform) {
      setConfirmRemoveTransform(true);
      return;
    }
    setConfirmRemoveTransform(false);
    setTransformModelBusy(true);
    setTransformModelError(null);
    try {
      await removeTransformModel();
      await refreshTransformModel();
    } catch (e) {
      setTransformModelError(String(e));
    } finally {
      setTransformModelBusy(false);
    }
  };

  const resetTransform = async () => {
    setTransformModelBusy(true);
    setTransformModelError(null);
    try {
      await resetTransformRuntime();
      await refreshTransformModel();
    } catch (e) {
      setTransformModelError(String(e));
    } finally {
      setTransformModelBusy(false);
    }
  };

  return {
    transformModel,
    transformModelBusy,
    transformModelError,
    confirmRemoveTransform,
    setConfirmRemoveTransform,
    transformKeyError,
    transformDownloadPct,
    updateTransformHoldKey,
    downloadTransform,
    removeTransform,
    resetTransform,
  };
}
