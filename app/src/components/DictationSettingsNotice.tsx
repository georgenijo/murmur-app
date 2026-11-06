import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '../lib/targetedEvents';
import { MainErrorBanner } from './MainErrorBanner';

const UNSAVED_MESSAGE = 'Murmur could not save the enabled/disabled setting. The tray checkmark shows the current runtime state. Restore settings storage before quitting to keep your choice.';
const UNAVAILABLE_MESSAGE = 'Murmur could not verify whether the enabled/disabled setting was saved. The tray checkmark shows the current runtime state.';

/** Independent of settings hydration and recording state, including recovery. */
export function DictationSettingsNotice() {
  const [message, setMessage] = useState('');
  useEffect(() => {
    let cancelled = false;
    let request = 0;
    const refresh = async () => {
      const current = ++request;
      try {
        const unsaved = await invoke<boolean>('get_dictation_settings_unsaved');
        if (!cancelled && current === request) setMessage(unsaved ? UNSAVED_MESSAGE : '');
      } catch {
        if (!cancelled && current === request) setMessage(UNAVAILABLE_MESSAGE);
      }
    };
    // The subscription acknowledgement precedes the initial query. Events only
    // invalidate: overlapping reads cannot restore an older warning or clear.
    const subscription = listen('dictation-settings-warning-changed', () => { void refresh(); });
    void subscription.then(() => { if (!cancelled) void refresh(); }, () => {
      if (!cancelled) void refresh();
    });
    return () => {
      cancelled = true;
      void subscription.then(unlisten => unlisten(), () => {});
    };
  }, []);
  return message ? <MainErrorBanner placement="window" message={message} onDismiss={() => setMessage('')} /> : null;
}
