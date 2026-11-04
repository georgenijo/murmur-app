import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS } from '../settings';

const mocks = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
  invoke: vi.fn(async () => undefined),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    mocks.handlers.set(name, handler);
    return () => { mocks.handlers.delete(name); };
  }),
}));
import { useCombinedToggle } from './useCombinedToggle';
import { useDoubleTapToggle } from './useDoubleTapToggle';
import { useHoldDownToggle } from './useHoldDownToggle';

describe('keyboard gesture event payloads', () => {
  it.each([
    ['hold', 'hold-down-start'],
    ['double', 'double-tap-toggle'],
    ['both', 'hold-down-start'],
    ['both', 'double-tap-toggle'],
  ])('passes the %s %s token without keeping it for the next event', async (mode, eventName) => {
    mocks.handlers.clear();
    const start = vi.fn();
    const toggle = vi.fn();
    const stop = vi.fn();
    const container = document.createElement('div');
    const root = createRoot(container);
    function Harness() {
      const common = { initialized: true, accessibilityGranted: true, triggerKey: DEFAULT_SETTINGS.doubleTapKey,
        doubleTapKey: DEFAULT_SETTINGS.doubleTapKey, holdDownKey: DEFAULT_SETTINGS.doubleTapKey,
        status: 'idle' as const, onStart: start, onStop: stop, onToggle: toggle };
      useHoldDownToggle({ ...common, enabled: mode === 'hold' });
      useDoubleTapToggle({ ...common, enabled: mode === 'double' });
      useCombinedToggle({ ...common, enabled: mode === 'both' });
      return null;
    }
    try {
      await act(async () => root.render(<Harness />));
      const callback = mocks.handlers.get(eventName);
      expect(callback).toBeDefined();
      await act(async () => callback?.({ payload: { gestureToken: '9007199254740993' } }));
      await act(async () => callback?.({ payload: { gestureToken: null } }));
      const received = eventName === 'hold-down-start' ? start : toggle;
      expect(received).toHaveBeenNthCalledWith(1, '9007199254740993');
      expect(received).toHaveBeenNthCalledWith(2, undefined);
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
});
