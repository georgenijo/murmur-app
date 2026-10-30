import { invoke } from '@tauri-apps/api/core';

/** macOS System Audio Recording consent the browser-media pause needs. */
export type BrowserMediaPermission = 'granted' | 'denied' | 'not_determined' | 'unavailable';

/** Reads consent without prompting. */
export async function getBrowserMediaPermission(): Promise<BrowserMediaPermission> {
  return invoke('get_browser_media_permission');
}

/** Shows the macOS prompt when undecided. Call only when the user turns the setting on. */
export async function requestBrowserMediaPermission(): Promise<BrowserMediaPermission> {
  return invoke('request_browser_media_permission');
}
