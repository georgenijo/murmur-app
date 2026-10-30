import { useEffect, useState } from 'react';
import {
  getBrowserMediaPermission,
  requestBrowserMediaPermission,
  type BrowserMediaPermission,
} from '../../lib/browserMedia';
import { openSystemAudioPreferences } from '../../lib/meetings';
import { SettingToggle } from './SettingToggle';

interface PauseBrowserMediaSettingProps {
  enabled: boolean;
  onEnabledChange: (enabled: boolean) => void;
}

const PERMISSION_TEXT: Record<BrowserMediaPermission, string> = {
  granted: 'System Audio Recording is allowed.',
  denied: 'Allow Murmur under Screen & System Audio Recording so it can hear whether a browser is playing. Until then nothing is paused.',
  not_determined: 'Waiting for System Audio Recording permission. Until it is allowed nothing is paused.',
  unavailable: 'Not available on this Mac.',
};

/** Off by default. The macOS prompt appears only when the user turns this on, never while dictating. */
export function PauseBrowserMediaSetting({ enabled, onEnabledChange }: PauseBrowserMediaSettingProps) {
  const [permission, setPermission] = useState<BrowserMediaPermission | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    const refresh = () => {
      void getBrowserMediaPermission()
        .then((next) => { if (!disposed) setPermission(next); })
        .catch(() => { if (!disposed) setPermission('unavailable'); });
    };
    refresh();
    // Returning from System Settings is the usual way consent changes.
    window.addEventListener('focus', refresh);
    return () => {
      disposed = true;
      window.removeEventListener('focus', refresh);
    };
  }, [enabled]);

  const toggle = () => {
    const next = !enabled;
    onEnabledChange(next);
    if (!next) return;
    void requestBrowserMediaPermission()
      .then(setPermission)
      .catch(() => setPermission('unavailable'));
  };

  return (
    <div data-setting-target="pause-browser-media" className="settings-stack rounded-lg transition-shadow [&.settings-target-flash]:ring-2 [&.settings-target-flash]:ring-primary/40">
      <SettingToggle
        title="Pause Browser Media While Dictating"
        description="Pause a video or song playing in Safari, Chrome, Arc, Brave, or Edge when you start dictating, and resume it afterwards. Murmur resumes only media it paused, never media you paused yourself."
        checked={enabled}
        onChange={toggle}
      />
      {enabled && permission && (
        <div className="flex items-center gap-2 text-xs text-on-surface-variant" role="status">
          <span>{PERMISSION_TEXT[permission]}</span>
          {permission === 'denied' && (
            <button type="button" onClick={() => { void openSystemAudioPreferences(); }} className="ml-auto underline">
              Open Settings
            </button>
          )}
        </div>
      )}
    </div>
  );
}
