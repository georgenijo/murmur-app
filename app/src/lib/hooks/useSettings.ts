import { useState, useRef, useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
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

  // Listen for settings changes from other windows (e.g. overlay quick-settings panel)
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<Partial<Settings>>('settings-changed', () => {
      // Re-read from localStorage to get the latest state
      const fresh = loadSettings();
      const prev = settingsRef.current;
      settingsRef.current = fresh;
      setSettings(fresh);
      // Re-configure backend if configure-relevant fields changed
      if (fresh.autoPaste !== prev.autoPaste || fresh.autoPasteDelayMs !== prev.autoPasteDelayMs ||
          fresh.model !== prev.model || fresh.language !== prev.language ||
          fresh.vadSensitivity !== prev.vadSensitivity || fresh.idleTimeoutMinutes !== prev.idleTimeoutMinutes ||
          fresh.customVocabulary !== prev.customVocabulary) {
        configure({
          model: fresh.model, language: fresh.language, autoPaste: fresh.autoPaste,
          autoPasteDelayMs: fresh.autoPasteDelayMs, vadSensitivity: fresh.vadSensitivity,
          idleTimeoutMinutes: fresh.idleTimeoutMinutes, customVocabulary: fresh.customVocabulary,
        }).catch((err) => console.error('Failed to reconfigure after settings-changed:', err));
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
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

    if ('model' in updates || 'language' in updates || 'autoPaste' in updates || 'autoPasteDelayMs' in updates || 'vadSensitivity' in updates || 'idleTimeoutMinutes' in updates || 'customVocabulary' in updates) {
      const version = ++configureVersionRef.current;
      configure({ model: newSettings.model, language: newSettings.language, autoPaste: newSettings.autoPaste, autoPasteDelayMs: newSettings.autoPasteDelayMs, vadSensitivity: newSettings.vadSensitivity, idleTimeoutMinutes: newSettings.idleTimeoutMinutes, customVocabulary: newSettings.customVocabulary })
        .catch((err) => {
          console.error('Failed to configure:', err);
          if (configureVersionRef.current === version) {
            const reverted = {
              ...settingsRef.current,
              model: previousSettings.model,
              language: previousSettings.language,
              autoPaste: previousSettings.autoPaste,
              autoPasteDelayMs: previousSettings.autoPasteDelayMs,
              vadSensitivity: previousSettings.vadSensitivity,
              idleTimeoutMinutes: previousSettings.idleTimeoutMinutes,
              customVocabulary: previousSettings.customVocabulary,
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
