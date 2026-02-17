import { useState, useEffect } from 'react';
import { initDictation, configure } from '../dictation';
import { Settings } from '../settings';

export function useInitialization(settings: Settings) {
  const [initialized, setInitialized] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    initDictation()
      .then(() =>
        configure({ model: settings.model, language: settings.language, autoPaste: settings.autoPaste })
      )
      .then(() => setInitialized(true))
      .catch((err) => setError(String(err)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // Only run once on mount — settings are loaded synchronously before this runs

  return { initialized, error };
}
