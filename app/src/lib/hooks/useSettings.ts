import { useState, useRef, useEffect } from 'react';
import { Settings, loadSettings, saveSettings } from '../settings';
import { configure } from '../dictation';
import { enable, disable, isEnabled } from '@tauri-apps/plugin-autostart';

let lastAutostartOp: Promise<void> = Promise.resolve();

export function useSettings() {
  const [settings, setSettings] = useState<Settings>(() => loadSettings());
  const settingsRef = useRef(settings);
  const configureVersionRef = useRef(0);

  // Sync launchAtLogin with OS state on mount.
  // Handles the case where a user removed the login item from System Settings.
  useEffect(() => {
    const initialLaunch = settingsRef.current.launchAtLogin;
    isEnabled().then((osEnabled) => {
      if (settingsRef.current.launchAtLogin === initialLaunch && osEnabled !== initialLaunch) {
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
      const attemptedValue = newSettings.launchAtLogin;
      const action = attemptedValue ? enable : disable;
      lastAutostartOp = lastAutostartOp.then(() => action()).catch((err) => {
        console.error('Failed to update autostart:', err);
        if (settingsRef.current.launchAtLogin === attemptedValue) {
          const reverted = { ...settingsRef.current, launchAtLogin: previousSettings.launchAtLogin };
          settingsRef.current = reverted;
          setSettings(reverted);
          saveSettings(reverted);
        }
      });
    }

    if ('model' in updates || 'language' in updates || 'autoPaste' in updates) {
      const version = ++configureVersionRef.current;
      configure({ model: newSettings.model, language: newSettings.language, autoPaste: newSettings.autoPaste })
        .catch((err) => {
          console.error('Failed to configure:', err);
          if (configureVersionRef.current === version) {
            const reverted = {
              ...settingsRef.current,
              model: previousSettings.model,
              language: previousSettings.language,
              autoPaste: previousSettings.autoPaste,
            };
            settingsRef.current = reverted;
            setSettings(reverted);
            saveSettings(reverted);
          }
        });
    }
  };

  return { settings, updateSettings };
}
