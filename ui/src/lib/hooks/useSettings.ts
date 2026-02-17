import { useState, useRef } from 'react';
import { Settings, loadSettings, saveSettings } from '../settings';
import { configure } from '../dictation';

export function useSettings() {
  const [settings, setSettings] = useState<Settings>(() => loadSettings());
  const settingsRef = useRef(settings);

  const updateSettings = async (updates: Partial<Settings>) => {
    const newSettings = { ...settingsRef.current, ...updates };
    settingsRef.current = newSettings;
    setSettings(newSettings);
    saveSettings(newSettings);

    if (updates.model || updates.language || updates.autoPaste !== undefined) {
      try {
        await configure({ model: newSettings.model, language: newSettings.language, autoPaste: newSettings.autoPaste });
      } catch (err) {
        console.error('Failed to configure:', err);
      }
    }
  };

  return { settings, updateSettings };
}
