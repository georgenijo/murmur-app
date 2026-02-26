import { useState, useRef, useEffect } from 'react';
import { Settings, loadSettings, saveSettings } from '../settings';
import { configure } from '../dictation';
import { enable, disable, isEnabled } from '@tauri-apps/plugin-autostart';

export function useSettings() {
  const [settings, setSettings] = useState<Settings>(() => loadSettings());
  const settingsRef = useRef(settings);
  const configureVersionRef = useRef(0);

  // Sync launchAtLogin with OS state on mount.
  // Handles the case where a user removed the login item from System Settings.
  useEffect(() => {
    isEnabled().then((osEnabled) => {
      if (osEnabled !== settingsRef.current.launchAtLogin) {
        const synced = { ...settingsRef.current, launchAtLogin: osEnabled };
        settingsRef.current = synced;
        setSettings(synced);
        saveSettings(synced);
      }
    }).catch((err) => {
      console.error('Failed to check autostart status:', err);
    });
  }, []);

  const updateSettings = (updates: Partial<Settings>) => {
    const previousSettings = settingsRef.current;
    const newSettings = { ...previousSettings, ...updates };
    settingsRef.current = newSettings;
    setSettings(newSettings);
    saveSettings(newSettings);

    if ('launchAtLogin' in updates) {
      const action = newSettings.launchAtLogin ? enable() : disable();
      action.catch((err) => {
        console.error('Failed to update autostart:', err);
        settingsRef.current = previousSettings;
        setSettings(previousSettings);
        saveSettings(previousSettings);
      });
    }

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
