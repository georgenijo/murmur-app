import { register, unregister } from '@tauri-apps/plugin-global-shortcut';

export type HotkeyCallback = () => void;

let currentHotkey: string | null = null;
let lastTriggerTime = 0;
const DEBOUNCE_MS = 300; // Prevent double-firing within 300ms

export async function registerHotkey(shortcut: string, callback: HotkeyCallback): Promise<void> {
  // Unregister previous hotkey if exists
  if (currentHotkey) {
    try {
      await unregister(currentHotkey);
    } catch (e) {
      console.warn('Failed to unregister previous hotkey:', e);
    }
  }

  // Wrap callback with debounce to prevent double-firing
  const debouncedCallback = () => {
    const now = Date.now();
    if (now - lastTriggerTime < DEBOUNCE_MS) {
      return;
    }
    lastTriggerTime = now;
    callback();
  };

  // Register new hotkey
  await register(shortcut, debouncedCallback);
  currentHotkey = shortcut;
}

export async function unregisterHotkey(): Promise<void> {
  if (currentHotkey) {
    await unregister(currentHotkey);
    currentHotkey = null;
  }
}

// Convert our hotkey option to Tauri shortcut format
// Note: Since we can't detect left/right modifier keys separately with global shortcuts,
// we use modifier+Space combinations as toggle shortcuts
export function hotkeyToShortcut(hotkey: string): string {
  const map: Record<string, string> = {
    'alt_r': 'Alt+Space',      // Use Alt+Space as toggle
    'alt_l': 'Alt+Space',
    'shift_r': 'Shift+Space',
    'shift_l': 'Shift+Space',
    'ctrl_r': 'Ctrl+Space',
  };
  return map[hotkey] || 'Alt+Space';
}
