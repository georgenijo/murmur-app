import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface PermissionStatus {
  microphone: 'unknown' | 'granted' | 'denied';
  accessibility: 'unknown' | 'granted' | 'denied';
}

export function PermissionsBanner() {
  const [permissions, setPermissions] = useState<PermissionStatus>({
    microphone: 'unknown',
    accessibility: 'unknown',
  });
  const [dismissed, setDismissed] = useState(false);
  const [checking, setChecking] = useState(true);

  const checkPermissions = async () => {
    setChecking(true);
    try {
      // Check accessibility permission via Tauri command
      const hasAccessibility = await invoke<boolean>('check_accessibility_permission');

      // Check microphone by attempting to get user media
      let hasMicrophone = false;
      try {
        if (navigator.mediaDevices?.getUserMedia) {
          const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
          stream.getTracks().forEach(track => track.stop());
          hasMicrophone = true;
        }
      } catch {
        hasMicrophone = false;
      }

      setPermissions({
        microphone: hasMicrophone ? 'granted' : 'denied',
        accessibility: hasAccessibility ? 'granted' : 'denied',
      });
    } catch (error) {
      console.error('Failed to check permissions:', error);
    } finally {
      setChecking(false);
    }
  };

  useEffect(() => {
    checkPermissions();

    // Re-check when window gains focus (user might have granted permission)
    const handleFocus = () => checkPermissions();
    window.addEventListener('focus', handleFocus);
    return () => window.removeEventListener('focus', handleFocus);
  }, []);

  const handleOpenAccessibility = async () => {
    await invoke('request_accessibility_permission');
  };

  const handleOpenMicrophone = async () => {
    await invoke('request_microphone_permission');
  };

  const allGranted = permissions.microphone === 'granted' && permissions.accessibility === 'granted';

  if (dismissed || allGranted || checking) {
    return null;
  }

  return (
    <div className="bg-amber-50 dark:bg-amber-900/30 border-b border-amber-200 dark:border-amber-800 px-4 py-3">
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1">
          <h3 className="text-sm font-medium text-amber-800 dark:text-amber-200">
            Permissions Required
          </h3>
          <div className="mt-2 space-y-2">
            {/* Microphone Permission */}
            <div className="flex items-center gap-2">
              <span className={`w-2 h-2 rounded-full ${
                permissions.microphone === 'granted'
                  ? 'bg-green-500'
                  : 'bg-red-500'
              }`} />
              <span className="text-sm text-amber-700 dark:text-amber-300">
                Microphone: {permissions.microphone === 'granted' ? 'Granted' : 'Required for recording'}
              </span>
              {permissions.microphone !== 'granted' && (
                <button
                  onClick={handleOpenMicrophone}
                  className="text-xs text-amber-600 dark:text-amber-400 underline hover:no-underline"
                >
                  Open Settings
                </button>
              )}
            </div>

            {/* Accessibility Permission */}
            <div className="flex items-center gap-2">
              <span className={`w-2 h-2 rounded-full ${
                permissions.accessibility === 'granted'
                  ? 'bg-green-500'
                  : 'bg-red-500'
              }`} />
              <span className="text-sm text-amber-700 dark:text-amber-300">
                Accessibility: {permissions.accessibility === 'granted' ? 'Granted' : 'Required for text pasting'}
              </span>
              {permissions.accessibility !== 'granted' && (
                <button
                  onClick={handleOpenAccessibility}
                  className="text-xs text-amber-600 dark:text-amber-400 underline hover:no-underline"
                >
                  Open Settings
                </button>
              )}
            </div>
          </div>

          <button
            onClick={checkPermissions}
            className="mt-2 text-xs text-amber-600 dark:text-amber-400 hover:underline"
          >
            Re-check permissions
          </button>
        </div>

        <button
          onClick={() => setDismissed(true)}
          className="text-amber-500 hover:text-amber-700 dark:hover:text-amber-300"
          aria-label="Dismiss"
        >
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
    </div>
  );
}
