import { invoke } from '@tauri-apps/api/core';

/** Show and focus the main window after a permission round-trip. */
export function showMainWindow(): Promise<void> {
  return invoke<void>('show_main_window');
}
