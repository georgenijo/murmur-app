import { useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

/** Native show/hide and minimization are distinct from focus and DOM visibility. */
export function useNativeWindowVisible(): boolean {
  // Persistent native windows can start hidden. Do no diagnostic work until
  // their native state has been read; the browser preview uses DOM visibility.
  const [visible, setVisible] = useState(() => !isTauri() && !document.hidden);

  useEffect(() => {
    if (!isTauri()) {
      const refresh = () => setVisible(!document.hidden);
      document.addEventListener('visibilitychange', refresh);
      return () => document.removeEventListener('visibilitychange', refresh);
    }

    const nativeWindow = getCurrentWindow();
    let disposed = false;
    let revision = 0;
    let closed = false;
    let settleTimer: ReturnType<typeof setTimeout> | undefined;
    const stops: Array<() => void> = [];
    const readVisibility = async () => {
      if (disposed || closed) return;
      const request = ++revision;
      try {
        const [shown, minimized] = await Promise.all([
          nativeWindow.isVisible(), nativeWindow.isMinimized(),
        ]);
        if (!disposed && request === revision && !closed) setVisible(shown && !minimized);
      } catch {
        if (!disposed && request === revision) setVisible(false);
      }
    };
    const refresh = (settleOnly = false) => {
      if (disposed || closed) return;
      revision += 1;
      if (!settleOnly) void readVisibility();
      // Focus/resize notifications can precede the native minimize/hide
      // transition. Recheck once after it settles, without an idle poller.
      clearTimeout(settleTimer);
      settleTimer = setTimeout(() => { void readVisibility(); }, 150);
    };
    const onDocumentVisibility = () => refresh();
    document.addEventListener('visibilitychange', onDocumentVisibility);

    const subscriptions = [
      nativeWindow.onFocusChanged(({ payload: focused }) => {
        if (focused) closed = false;
        refresh();
      }),
      // Resizing can emit at frame rate; debounce its native reads.
      nativeWindow.onResized(() => refresh(true)),
      // Observe the raw event: onCloseRequested() destroys the window unless
      // its callback prevents default. Rust already owns close-to-hide.
      nativeWindow.listen('tauri://close-requested', () => {
        // Rust turns close into hide. Invalidate any in-flight hydration now;
        // an old visible response must not reactivate the hidden workspace.
        closed = true;
        revision += 1;
        clearTimeout(settleTimer);
        if (!disposed) setVisible(false);
      }),
    ];
    void Promise.all(subscriptions.map(subscription => subscription.then(stop => {
      if (disposed) stop();
      else stops.push(stop);
    }).catch(() => undefined))).then(() => {
      // Subscribe before hydrating, including when listener registration is slow.
      if (!disposed) refresh();
    });

    return () => {
      disposed = true;
      revision += 1;
      clearTimeout(settleTimer);
      document.removeEventListener('visibilitychange', onDocumentVisibility);
      stops.forEach(stop => stop());
    };
  }, []);

  return visible;
}
