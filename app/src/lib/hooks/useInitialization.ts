import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { initDictation, configure, buildConfigureOptions } from '../dictation';
import { Settings } from '../settings';

export function useInitialization(getCurrentSettings: () => Settings) {
  const [initialized, setInitialized] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    let cancelled = false;
    initDictation()
      .then(() => {
        if (cancelled) return;
        return configure(() => buildConfigureOptions(getCurrentSettings()));
      })
      .then(() => {
        if (cancelled) return;
        return invoke('set_app_disabled', { disabled: getCurrentSettings().disabled }).catch(() => {});
      })
      .then(() => { if (!cancelled) setInitialized(true); })
      .catch((err) => { if (!cancelled) setError(String(err)); });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // Only run once on mount; read the latest settings when initialization finishes

  return { initialized, error };
}
