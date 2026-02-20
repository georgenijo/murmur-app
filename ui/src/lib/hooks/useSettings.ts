import { useState, useRef } from 'react';
import { Settings, loadSettings, saveSettings } from '../settings';
import { configure } from '../dictation';

export function useSettings() {
  const [settings, setSettings] = useState<Settings>(() => loadSettings());
  const settingsRef = useRef(settings);
  const configureVersionRef = useRef(0);

  const updateSettings = (updates: Partial<Settings>) => {
    const previousSettings = settingsRef.current;
    const newSettings = { ...previousSettings, ...updates };
    settingsRef.current = newSettings;
    setSettings(newSettings);
    saveSettings(newSettings);

    if ('model' in updates || 'language' in updates || 'autoPaste' in updates) {
      const version = ++configureVersionRef.current;
      configure({ model: newSettings.model, language: newSettings.language, autoPaste: newSettings.autoPaste })
        .catch((err) => {
          console.error('Failed to configure:', err);
          // Revert only if no newer configure has been requested since this one
          if (configureVersionRef.current === version) {
            settingsRef.current = previousSettings;
            setSettings(previousSettings);
            saveSettings(previousSettings);
          }
        });
    }
  };

  return { settings, updateSettings };
}
