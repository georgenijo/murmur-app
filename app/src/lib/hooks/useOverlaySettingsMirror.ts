import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import { loadSettings, saveSettings } from '../settings';
import type { Settings } from '../settings';
import { buildConfigureOptions } from '../dictation';

export interface UseOverlaySettingsMirrorArgs {
  setDisabled: (value: boolean) => void;
  setShowHotkeyMiss: (value: boolean) => void;
  /** Shared with useOverlayRuntime — see that hook's doc comment. */
  hotkeyMissFeedbackRef: React.MutableRefObject<boolean>;
}

export interface OverlaySettingsMirror {
  autoPaste: boolean;
  fileOutputEnabled: boolean;
  /** Re-reads localStorage and applies the snapshot. Stable identity. */
  refresh: () => void;
  handleToggleAutoPaste: (e: React.MouseEvent) => Promise<void>;
  handleToggleDisabled: (e: React.MouseEvent) => Promise<void>;
  handleOpenSettings: (e: React.MouseEvent) => Promise<void>;
}

/**
 * Mirrors the subset of localStorage Settings the overlay's quick controls
 * need, since the overlay is a separate webview with no shared React settings
 * context. Applies on mount, on `settings-changed`, and (via the exposed
 * `refresh`) whenever the composition shell decides the dropdown is about to
 * become visible (`phase === 'opening'`) — wiring that stays in
 * OverlayWidget.tsx because `phase` comes from the expansion controller.
 */
export function useOverlaySettingsMirror({
  setDisabled,
  setShowHotkeyMiss,
  hotkeyMissFeedbackRef,
}: UseOverlaySettingsMirrorArgs): OverlaySettingsMirror {
  const [autoPaste, setAutoPaste] = useState(false);
  const [fileOutputEnabled, setFileOutputEnabled] = useState(false);

  const applySettingsSnapshot = useCallback((settings: Settings) => {
    setDisabled(settings.disabled);
    setAutoPaste(settings.autoPaste);
    setFileOutputEnabled(settings.saveTranscript || settings.saveAudio);
    hotkeyMissFeedbackRef.current = settings.hotkeyMissFeedback;
    if (!settings.hotkeyMissFeedback) setShowHotkeyMiss(false);
  }, [setDisabled, setShowHotkeyMiss, hotkeyMissFeedbackRef]);

  const refresh = useCallback(() => {
    try {
      applySettingsSnapshot(loadSettings());
    } catch { /* ignore */ }
  }, [applySettingsSnapshot]);

  // Read initial settings once on mount (geometry is owned by useOverlayGeometry).
  useEffect(() => {
    refresh();
  }, [refresh]);

  // Subscribe to settings-changed (emitted by the main window) so the quick
  // controls reflect changes made there, even while already expanded.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen('settings-changed', () => {
      refresh();
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [refresh]);

  // Quick control: auto-paste. Write localStorage + notify the main window.
  const handleToggleAutoPaste = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      const s = loadSettings();
      const next = !s.autoPaste;
      const nextSettings = { ...s, autoPaste: next };
      saveSettings(nextSettings);
      applySettingsSnapshot(nextSettings);
      try {
        await invoke('configure_dictation', { options: buildConfigureOptions(nextSettings) });
      } catch (err) {
        saveSettings(s);
        applySettingsSnapshot(s);
        throw err;
      }
      emit('settings-changed').catch((err) => flog.warn('overlay', 'emit settings-changed failed', { error: String(err) }));
    } catch (err) {
      flog.error('overlay', 'toggle autoPaste failed', { error: String(err) });
      try {
        applySettingsSnapshot(loadSettings());
      } catch { /* ignore */ }
    }
  }, [applySettingsSnapshot]);

  // Quick control: global disable. Gate the backend immediately, then notify.
  const handleToggleDisabled = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      const s = loadSettings();
      const next = !s.disabled;
      await invoke('set_app_disabled', { disabled: next });
      saveSettings({ ...s, disabled: next });
      setDisabled(next);
      emit('settings-changed').catch((err) => flog.warn('overlay', 'emit settings-changed failed', { error: String(err) }));
    } catch (err) {
      flog.error('overlay', 'toggle disabled failed', { error: String(err) });
    }
  }, [setDisabled]);

  // Quick control: open the main window's Settings panel.
  const handleOpenSettings = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await invoke('show_main_window');
      await emit('open-settings');
    } catch (err) {
      flog.error('overlay', 'open settings failed', { error: String(err) });
    }
  }, []);

  return {
    autoPaste,
    fileOutputEnabled,
    refresh,
    handleToggleAutoPaste,
    handleToggleDisabled,
    handleOpenSettings,
  };
}
