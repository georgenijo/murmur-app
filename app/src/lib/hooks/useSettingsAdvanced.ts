import { useCallback, useRef, useState } from 'react';

/** UI-only state. Never put settings, paths, permissions or editor drafts here. */
export const SETTINGS_ADVANCED_KEY = 'murmur.settings-disclosure.v1';
export const SETTINGS_PANE_KEYS = [
  'general', 'shortcuts', 'recording', 'customize', 'delivery', 'meetings', 'text',
  'ai', 'ai-query', 'ai-transform', 'ai-transcription', 'modes', 'appearance',
  'performance', 'diagnostics', 'vocabulary', 'aliases', 'knowledge', 'transforms', 'commands', 'scan', 'app-overrides',
] as const;
export type SettingsPaneKey = typeof SETTINGS_PANE_KEYS[number];
type AdvancedState = Partial<Record<SettingsPaneKey, boolean>>;

export function readSettingsAdvanced(): AdvancedState {
  try {
    const raw = localStorage.getItem(SETTINGS_ADVANCED_KEY);
    if (!raw || raw.length > 2048) return {};
    const value: unknown = JSON.parse(raw);
    if (!value || typeof value !== 'object' || !('version' in value) || value.version !== 1
      || !('panes' in value) || !value.panes || typeof value.panes !== 'object' || Array.isArray(value.panes)) return {};
    const panes = value.panes as Record<string, unknown>;
    return Object.fromEntries(SETTINGS_PANE_KEYS.filter(key => typeof panes[key] === 'boolean').map(key => [key, panes[key]]));
  } catch { return {}; }
}

export function useSettingsAdvanced(pane: string) {
  const [state, setState] = useState(readSettingsAdvanced);
  const stateRef = useRef(state);
  const open = state[pane as SettingsPaneKey] === true;
  const setOpen = useCallback((next: boolean) => {
    if (!(SETTINGS_PANE_KEYS as readonly string[]).includes(pane)) return;
    // Merge with disk so another pane/window cannot overwrite newer UI state.
    const panes = { ...stateRef.current, ...readSettingsAdvanced(), [pane]: next };
    stateRef.current = panes;
    try { localStorage.setItem(SETTINGS_ADVANCED_KEY, JSON.stringify({ version: 1, panes })); } catch { /* Session state still works if storage is unavailable. */ }
    setState(panes);
  }, [pane]);
  return [open, setOpen] as const;
}
