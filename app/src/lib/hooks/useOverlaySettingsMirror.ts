import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import {
  loadSettings,
  parseSettingsBlob,
  smartAutoMicrophoneRequest,
} from '../settings';
import type { Settings, SmartAutoMicrophoneRequest } from '../settings';

export interface UseOverlaySettingsMirrorArgs {
  setDisabled: (value: boolean) => void;
  setShowHotkeyMiss: (value: boolean) => void;
  /** Shared with useOverlayRuntime — see that hook's doc comment. */
  hotkeyMissFeedbackRef: React.MutableRefObject<boolean>;
}

export interface OverlaySettingsMirror {
  autoPaste: boolean;
  fileOutputEnabled: boolean;
  overlayVerticalOffset: number;
  overlayPlacement: Settings['overlayPlacement'];
  overlayProcessingStyle: Settings['overlayProcessingStyle'];
  smartAuto: SmartAutoMicrophoneRequest | null;
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
  const durableSnapshot = useRef<Settings | null>(null);
  const refreshDurableRef = useRef<() => Promise<void>>(async () => {});
  const autoPasteOperation = useRef(0);
  const disabledOperation = useRef(0);
  const [fileOutputEnabled, setFileOutputEnabled] = useState(false);
  const [overlayVerticalOffset, setOverlayVerticalOffset] = useState(
    () => loadSettings().overlayVerticalOffset,
  );
  const [overlayPlacement, setOverlayPlacement] = useState(() => loadSettings().overlayPlacement);
  const [overlayProcessingStyle, setOverlayProcessingStyle] = useState(() => loadSettings().overlayProcessingStyle);
  const [smartAuto, setSmartAuto] = useState<SmartAutoMicrophoneRequest | null>(
    () => smartAutoMicrophoneRequest(loadSettings()),
  );

  const applySettingsSnapshot = useCallback((settings: Settings) => {
    if (durableSnapshot.current !== null) durableSnapshot.current = settings;
    setDisabled(settings.disabled);
    setAutoPaste(settings.autoPaste);
    setFileOutputEnabled(settings.saveTranscript || settings.saveAudio);
    setOverlayVerticalOffset(settings.overlayVerticalOffset);
    setOverlayPlacement(settings.overlayPlacement);
    setOverlayProcessingStyle(settings.overlayProcessingStyle);
    setSmartAuto(smartAutoMicrophoneRequest(settings));
    hotkeyMissFeedbackRef.current = settings.hotkeyMissFeedback;
    if (!settings.hotkeyMissFeedback) setShowHotkeyMiss(false);
  }, [setDisabled, setShowHotkeyMiss, hotkeyMissFeedbackRef]);

  const refresh = useCallback(() => {
    try {
      applySettingsSnapshot(durableSnapshot.current ?? loadSettings());
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

  // A hidden/independent WKWebView may receive the generic notification before
  // its localStorage cache changes. Read the committed file without mutating a
  // shared cache; older reads must not replace a newer snapshot.
  useEffect(() => {
    let cancelled = false, revision = 0;
    let unlisten: (() => void) | undefined;
    const refreshDurable = async () => {
      const request = ++revision;
      try {
        const blob = await invoke<string | null>('load_settings_blob');
        if (cancelled || request !== revision || typeof blob !== 'string') return;
        const settings = parseSettingsBlob(blob);
        durableSnapshot.current = settings;
        applySettingsSnapshot(settings);
      } catch { /* keep the confirmed/optimistic snapshot on an unavailable bridge */ }
    };
    refreshDurableRef.current = refreshDurable;
    void listen('overlay-settings-persisted', () => { void refreshDurable(); }).then(fn => {
      if (cancelled) fn(); else { unlisten = fn; void refreshDurable(); }
    }).catch(() => {});
    return () => { cancelled = true; revision++; refreshDurableRef.current = async () => {}; unlisten?.(); };
  }, [applySettingsSnapshot]);

  // Persist only the owned field; the host merges into the latest durable
  // document under its writer lock. Never write a retained settings snapshot.
  const commitOwnedField = useCallback(async (field: 'autoPaste' | 'disabled', value: boolean) => {
    const blob = await invoke<string>('patch_overlay_setting', { field, value });
    durableSnapshot.current = parseSettingsBlob(blob);
    applySettingsSnapshot(durableSnapshot.current);
    // Invalidate reads started before this acknowledgement and catch commits
    // made since the patch, including a concurrent main-window opt-out.
    await refreshDurableRef.current();
  }, [applySettingsSnapshot]);

  const handleToggleAutoPaste = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    const operation = ++autoPasteOperation.current;
    try {
      const next = !(durableSnapshot.current ?? loadSettings()).autoPaste;
      await invoke('configure_dictation', { options: { autoPaste: next } });
      if (operation !== autoPasteOperation.current) return;
      await commitOwnedField('autoPaste', next);
    } catch (err) {
      flog.error('overlay', 'toggle autoPaste failed', { error: String(err) });
      await refreshDurableRef.current();
    }
  }, [commitOwnedField]);

  const handleToggleDisabled = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    const operation = ++disabledOperation.current;
    try {
      const next = !(durableSnapshot.current ?? loadSettings()).disabled;
      await invoke('set_app_disabled', { disabled: next });
      if (operation !== disabledOperation.current) return;
      await commitOwnedField('disabled', next);
    } catch (err) {
      flog.error('overlay', 'toggle disabled failed', { error: String(err) });
      await refreshDurableRef.current();
    }
  }, [commitOwnedField]);

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
    overlayVerticalOffset,
    overlayPlacement,
    overlayProcessingStyle,
    smartAuto,
    refresh,
    handleToggleAutoPaste,
    handleToggleDisabled,
    handleOpenSettings,
  };
}
