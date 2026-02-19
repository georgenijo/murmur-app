import { useState, useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';

export function useShowAboutListener() {
  const [showAbout, setShowAbout] = useState(false);

  useEffect(() => {
    const unlisten = listen('show-about', () => {
      setShowAbout(true);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  return { showAbout, setShowAbout };
}
