import { invoke } from '@tauri-apps/api/core';

interface PermissionsBannerProps {
  onDismiss: () => void;
}

export function PermissionsBanner({ onDismiss }: PermissionsBannerProps) {
  const openSystemPrefs = async () => {
    // Open System Preferences > Privacy & Security
    try {
      await invoke('open_system_preferences');
    } catch (e) {
      // Fallback: just show instructions
      console.error('Failed to open System Preferences:', e);
    }
  };

  return (
    <div className="bg-yellow-50 dark:bg-yellow-900/20 border-l-4 border-yellow-400 p-4 mb-4">
      <div className="flex items-start">
        <div className="flex-1">
          <h3 className="text-sm font-medium text-yellow-800 dark:text-yellow-200">
            Permissions Required
          </h3>
          <p className="mt-1 text-sm text-yellow-700 dark:text-yellow-300">
            For dictation and hotkeys to work, please enable:
          </p>
          <ul className="mt-2 text-sm text-yellow-700 dark:text-yellow-300 list-disc list-inside">
            <li>Microphone (for recording)</li>
            <li>Accessibility (for global hotkey)</li>
          </ul>
          <button
            onClick={openSystemPrefs}
            className="mt-3 text-sm font-medium text-yellow-800 dark:text-yellow-200 underline hover:no-underline"
          >
            Open System Settings
          </button>
        </div>
        <button
          onClick={onDismiss}
          className="ml-4 text-yellow-400 hover:text-yellow-500"
        >
          <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
            <path fillRule="evenodd" d="M4.293 4.293a1 1 0 011.414 0L10 8.586l4.293-4.293a1 1 0 111.414 1.414L11.414 10l4.293 4.293a1 1 0 01-1.414 1.414L10 11.414l-4.293 4.293a1 1 0 01-1.414-1.414L8.586 10 4.293 5.707a1 1 0 010-1.414z" clipRule="evenodd" />
          </svg>
        </button>
      </div>
    </div>
  );
}
