import { createContext, useContext } from 'react';

/**
 * Settings stays mounted behind the main surface for fast navigation. Context
 * lets visibility-sensitive children wake up without forcing the entire
 * memoized Settings tree to re-render on every open/close transition.
 */
export const SettingsSurfaceActiveContext = createContext(true);

export function useSettingsSurfaceActive(): boolean {
  return useContext(SettingsSurfaceActiveContext);
}
