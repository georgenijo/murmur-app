import { useState, useRef, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import { Settings, loadSettings, saveSettings } from '../settings';
import { configure, buildConfigureOptions } from '../dictation';
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

  // Persist backend-driven disabled changes (the tray's "Disable Murmur" item).
  // The equality guard makes this window's own set_app_disabled echo a no-op.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<boolean>('app-disabled-changed', (event) => {
      if (typeof event.payload !== 'boolean') return;
      const prev = settingsRef.current;
      if (prev.disabled === event.payload) return;
      const next = { ...prev, disabled: event.payload };
      settingsRef.current = next;
      setSettings(next);
      saveSettings(next);
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

    if ('disabled' in updates) {
      invoke('set_app_disabled', { disabled: newSettings.disabled }).catch((err) => {
        console.error('Failed to sync disabled state:', err);
      });
    }

    if ('autoPaste' in updates || 'disabled' in updates || 'saveTranscript' in updates || 'saveAudio' in updates || 'hotkeyMissFeedback' in updates) {
      // Notify the overlay window (separate React context) so its quick-settings
      // controls reflect changes made here. The diff-guard in applyExternalSettings
      // prevents this window from re-applying its own change.
      emit('settings-changed').catch((err) => console.error('Failed to emit settings-changed:', err));
    }

    if ('model' in updates || 'language' in updates || 'autoPaste' in updates || 'autoPasteDelayMs' in updates || 'vadSensitivity' in updates || 'idleTimeoutMinutes' in updates || 'customVocabulary' in updates || 'smartPunctuation' in updates || 'saveTranscript' in updates || 'saveAudio' in updates || 'outputDir' in updates || 'appProfiles' in updates || 'voiceCommandsEnabled' in updates || 'voiceCommands' in updates || 'cleanupEnabled' in updates || 'cleanupRemoveFiller' in updates || 'cleanupCapitalize' in updates || 'codeVocabEnabled' in updates || 'codeVocabFolder' in updates || 'correctionEnabled' in updates || 'correctionFuzzy' in updates) {
      const version = ++configureVersionRef.current;
      configure(buildConfigureOptions(newSettings))
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
              smartPunctuation: previousSettings.smartPunctuation,
              saveTranscript: previousSettings.saveTranscript,
              saveAudio: previousSettings.saveAudio,
              outputDir: previousSettings.outputDir,
              appProfiles: previousSettings.appProfiles,
              voiceCommandsEnabled: previousSettings.voiceCommandsEnabled,
              voiceCommands: previousSettings.voiceCommands,
              cleanupEnabled: previousSettings.cleanupEnabled,
              cleanupRemoveFiller: previousSettings.cleanupRemoveFiller,
              cleanupCapitalize: previousSettings.cleanupCapitalize,
              codeVocabEnabled: previousSettings.codeVocabEnabled,
              codeVocabFolder: previousSettings.codeVocabFolder,
              correctionEnabled: previousSettings.correctionEnabled,
              correctionFuzzy: previousSettings.correctionFuzzy,
            };
            settingsRef.current = reverted;
            setSettings(reverted);
            saveSettings(reverted);
          }
        });
    }
  };

  // Ingest a settings change made by another window (the overlay's quick controls).
  // Diffs against the current value so a window applying its own emitted change is a
  // no-op — this is what breaks the settings-changed echo loop.
  const applyExternalSettings = useCallback((fresh: Settings) => {
    const prev = settingsRef.current;
    const disabledChanged = fresh.disabled !== prev.disabled;
    const autoPasteChanged = fresh.autoPaste !== prev.autoPaste;
    if (!disabledChanged && !autoPasteChanged) return;

    settingsRef.current = fresh;
    setSettings(fresh);
    saveSettings(fresh);

    if (disabledChanged) {
      // Idempotent: the overlay also calls this directly for a snappy gate.
      invoke('set_app_disabled', { disabled: fresh.disabled }).catch((err) => {
        console.error('Failed to sync disabled state:', err);
      });
    }
    if (autoPasteChanged) {
      configure(buildConfigureOptions(fresh)).catch((err) => {
        console.error('Failed to configure:', err);
      });
    }
  }, []);

  return { settings, updateSettings, applyExternalSettings };
}
