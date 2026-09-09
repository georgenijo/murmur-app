import { useState, useRef, useEffect, useCallback } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import { Settings, loadSettings, saveSettings } from '../settings';
import { configure, buildConfigureOptions } from '../dictation';
import { enable, disable, isEnabled } from '@tauri-apps/plugin-autostart';
import {
  migrateLegacyMicrophoneId,
} from '../audioDevices';
import { INTERNAL_BENCHMARK_BUILD } from '../buildFlavor';
import { useAudioInputInventory } from './useAudioInputInventory';
import {
  configureSmartAutoProbe,
  smartAutoProbePolicy,
  type SmartAutoProbePolicy,
} from '../smartAutoMicrophone';

let lastAutostartOp: Promise<void> = Promise.resolve();

export function useSettings() {
  const [settings, setSettings] = useState<Settings>(() => loadSettings());
  const [configureError, setConfigureError] = useState<string | null>(null);
  const [probeConfigureError, setProbeConfigureError] = useState<string | null>(null);
  const settingsRef = useRef(settings);
  const configureVersionRef = useRef(0);
  const desiredProbePolicyRef = useRef<SmartAutoProbePolicy>(smartAutoProbePolicy(settings));
  const desiredProbePolicyVersionRef = useRef(0);
  const attemptedProbePolicyVersionRef = useRef(-1);
  const probeWriterRunningRef = useRef(false);
  const [microphoneMigrationPending, setMicrophoneMigrationPending] = useState(
    () => !settings.microphoneIdMigrationComplete,
  );
  const audioInventory = useAudioInputInventory(microphoneMigrationPending);

  const scheduleProbePolicyWrite = useCallback(() => {
    if (!isTauri() || probeWriterRunningRef.current) return;
    probeWriterRunningRef.current = true;
    void (async () => {
      while (attemptedProbePolicyVersionRef.current !== desiredProbePolicyVersionRef.current) {
        const version = desiredProbePolicyVersionRef.current;
        const policy = desiredProbePolicyRef.current;
        try {
          await configureSmartAutoProbe(policy);
        } catch {
          if (version === desiredProbePolicyVersionRef.current) {
            const current = settingsRef.current;
            if (policy.enabled && current.smartAutoProbeEnabled) {
              const disabled = { ...current, smartAutoProbeEnabled: false };
              settingsRef.current = disabled;
              setSettings(disabled);
              saveSettings(disabled);
              desiredProbePolicyRef.current = { enabled: false };
              desiredProbePolicyVersionRef.current += 1;
              void emit('settings-changed');
              setProbeConfigureError('Background microphone checks could not be enabled. Review approved microphones and try again.');
            } else {
              setProbeConfigureError('Background microphone checks could not be stopped. Quit Murmur before changing microphones.');
            }
          }
        }
        attemptedProbePolicyVersionRef.current = version;
      }
      probeWriterRunningRef.current = false;
    })();
  }, []);

  useEffect(() => {
    scheduleProbePolicyWrite();
  }, [scheduleProbePolicyWrite]);

  // Migrate pre-CPAL-0.18 display-name selections during app settings
  // initialization, not when Settings happens to be opened. Only a unique
  // display-name match is persisted as the backend-native stable ID;
  // ambiguous/missing names remain unresolved so the UI requires reselection.
  useEffect(() => {
    const inventory = audioInventory.inventory;
    if (!microphoneMigrationPending || inventory?.status !== 'available') return;
    const current = settingsRef.current;
    const microphone = migrateLegacyMicrophoneId(current.microphone, inventory.devices);
    const proven = inventory.devices.some((device) => device.id === current.microphone)
      || microphone !== current.microphone;
    if (!proven) return;
    setMicrophoneMigrationPending(false);
    const migrated = { ...current, microphone, microphoneIdMigrationComplete: true };
    settingsRef.current = migrated;
    setSettings(migrated);
    saveSettings(migrated);
  }, [audioInventory.inventory, microphoneMigrationPending]);

  // Sync launchAtLogin with OS state on mount.
  // Handles the case where a user removed the login item from System Settings.
  useEffect(() => {
    if (INTERNAL_BENCHMARK_BUILD) return;
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

  // Native overlay/tray cycling owns the immediate runtime transition. Mirror
  // manual selections into durable Settings without replaying the command.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<unknown>('mode-manual-changed', ({ payload }) => {
      const modeId = (payload as { modeId?: unknown } | null)?.modeId;
      if (typeof modeId !== 'string' || settingsRef.current.activeModeId === modeId) return;
      const next = { ...settingsRef.current, activeModeId: modeId };
      settingsRef.current = next;
      setSettings(next);
      saveSettings(next);
      void emit('settings-changed');
    }).then((fn) => {
      if (cancelled) fn(); else unlisten = fn;
    });
    return () => { cancelled = true; unlisten?.(); };
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

  const updateSettings = useCallback((updates: Partial<Settings>) => {
    setConfigureError(null);
    const previousSettings = settingsRef.current;
    const newSettings = {
      ...previousSettings,
      ...updates,
      ...('microphone' in updates ? { microphoneIdMigrationComplete: true } : {}),
    };
    settingsRef.current = newSettings;
    const probePolicyChanged = 'microphone' in updates
      || 'smartAutoMicrophoneEnabled' in updates
      || 'smartAutoProbeEnabled' in updates
      || 'smartAutoApprovedDeviceIds' in updates
      || 'smartAutoPreferredDeviceIds' in updates
      || 'smartAutoAllowContinuity' in updates;
    if (probePolicyChanged) {
      desiredProbePolicyRef.current = smartAutoProbePolicy(newSettings);
      desiredProbePolicyVersionRef.current += 1;
      setProbeConfigureError(null);
    }
    setSettings(newSettings);
    saveSettings(newSettings);

    if ('microphone' in updates && updates.microphone !== previousSettings.microphone) {
      // A picker-originated value is already based on an authoritative inventory.
      setMicrophoneMigrationPending(false);
      invoke('cancel_audio_initialization', { reason: 'device_changed' }).catch((err) => {
        console.error('Failed to cancel audio initialization after device change:', err);
      });
    }

    if ('launchAtLogin' in updates && !INTERNAL_BENCHMARK_BUILD) {
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

    if ('model' in updates || 'autoPaste' in updates || 'disabled' in updates || 'saveTranscript' in updates || 'saveAudio' in updates || 'hotkeyMissFeedback' in updates || 'overlayVerticalOffset' in updates || 'activeModeId' in updates || 'modes' in updates || 'appProfiles' in updates || 'siteModeLookupEnabled' in updates || 'browserSiteRules' in updates || probePolicyChanged) {
      // Notify the overlay window (separate React context) so its quick-settings
      // controls reflect changes made here. The diff-guard in applyExternalSettings
      // prevents this window from re-applying its own change.
      emit('settings-changed').catch((err) => console.error('Failed to emit settings-changed:', err));
    }

    if (probePolicyChanged) scheduleProbePolicyWrite();

    if ('model' in updates || 'language' in updates || 'autoPaste' in updates || 'autoPasteDelayMs' in updates || 'vadSensitivity' in updates || 'idleTimeoutMinutes' in updates || 'customVocabulary' in updates || 'vocabularyEntries' in updates || 'smartPunctuation' in updates || 'saveTranscript' in updates || 'saveAudio' in updates || 'mirrorToNotchPill' in updates || 'outputDir' in updates || 'appProfiles' in updates || 'modes' in updates || 'activeModeId' in updates || 'siteModeLookupEnabled' in updates || 'browserSiteRules' in updates || 'voiceCommandsEnabled' in updates || 'voiceCommands' in updates || 'cleanupEnabled' in updates || 'smartFormattingEnabled' in updates || 'cleanupRemoveFiller' in updates || 'cleanupCapitalize' in updates || 'codeVocabEnabled' in updates || 'codeVocabFolder' in updates || 'correctionEnabled' in updates || 'correctionFuzzy' in updates) {
      const version = ++configureVersionRef.current;
      configure(buildConfigureOptions(newSettings))
        .catch(() => {
          console.error('Failed to configure settings; previous values restored.');
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
              vocabularyEntries: previousSettings.vocabularyEntries,
              smartPunctuation: previousSettings.smartPunctuation,
              saveTranscript: previousSettings.saveTranscript,
              saveAudio: previousSettings.saveAudio,
              mirrorToNotchPill: previousSettings.mirrorToNotchPill,
              outputDir: previousSettings.outputDir,
              appProfiles: previousSettings.appProfiles,
              modes: previousSettings.modes,
              activeModeId: previousSettings.activeModeId,
              siteModeLookupEnabled: previousSettings.siteModeLookupEnabled,
              browserSiteRules: previousSettings.browserSiteRules,
              voiceCommandsEnabled: previousSettings.voiceCommandsEnabled,
              voiceCommands: previousSettings.voiceCommands,
              cleanupEnabled: previousSettings.cleanupEnabled,
              smartFormattingEnabled: previousSettings.smartFormattingEnabled,
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
            setConfigureError(
              'Settings could not be saved. Previous settings were restored. Check vocabulary aliases and Voice Commands for conflicts, then try again.',
            );
          }
        });
    }
  }, [scheduleProbePolicyWrite]);

  // Ingest a settings change made by another window (the overlay's quick controls).
  // Diffs against the current value so a window applying its own emitted change is a
  // no-op — this is what breaks the settings-changed echo loop.
  const applyExternalSettings = useCallback((fresh: Settings) => {
    const prev = settingsRef.current;
    const disabledChanged = fresh.disabled !== prev.disabled;
    const autoPasteChanged = fresh.autoPaste !== prev.autoPaste;
    if (!disabledChanged && !autoPasteChanged) return;

    // Overlay quick controls own these two fields only. Its cached approvals
    // must never replace a newer main-window microphone policy.
    const next = { ...prev, disabled: fresh.disabled, autoPaste: fresh.autoPaste };
    settingsRef.current = next;
    setSettings(next);
    saveSettings(next);

    if (disabledChanged) {
      // Idempotent: the overlay also calls this directly for a snappy gate.
      invoke('set_app_disabled', { disabled: fresh.disabled }).catch((err) => {
        console.error('Failed to sync disabled state:', err);
      });
    }
    if (autoPasteChanged) {
      configure(buildConfigureOptions(next)).catch(() => {
        console.error('Failed to configure externally changed settings.');
        setConfigureError('Settings could not be synchronized. Reopen Settings and try again.');
      });
    }
  }, []);

  return {
    settings,
    updateSettings,
    applyExternalSettings,
    configureError: probeConfigureError ?? configureError,
  };
}
