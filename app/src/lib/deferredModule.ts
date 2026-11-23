import { useEffect, useState } from 'react';

// A code-split module that renders synchronously once it has loaded. Unlike
// React.lazy, a preloaded module never suspends for a commit, so a panel opened
// after the background preload shows its content in the first frame. A failed
// import is not cached: the caller keeps its fallback and the next attempt
// retries, so a chunk error cannot unmount the surrounding window.
export interface DeferredModule<T> {
  current(): T | undefined;
  preload(): Promise<T>;
}

export function deferredModule<T>(load: () => Promise<T>): DeferredModule<T> {
  let value: T | undefined;
  let pending: Promise<T> | undefined;
  return {
    current: () => value,
    preload: () => {
      pending ??= load().then(
        (loaded) => {
          value = loaded;
          return loaded;
        },
        (error: unknown) => {
          pending = undefined;
          throw error;
        },
      );
      return pending;
    },
  };
}

/** Returns the module once loaded; `retryKey` changes retry a failed load. */
export function useDeferredModule<T>(module: DeferredModule<T>, retryKey?: unknown): T | undefined {
  const [loaded, setLoaded] = useState<T | undefined>(module.current);
  useEffect(() => {
    if (loaded) return;
    let live = true;
    module.preload().then(
      (value) => { if (live) setLoaded(() => value); },
      (error: unknown) => { console.error('Failed to load a main-window panel', error); },
    );
    return () => { live = false; };
  }, [module, loaded, retryKey]);
  return loaded ?? module.current();
}
