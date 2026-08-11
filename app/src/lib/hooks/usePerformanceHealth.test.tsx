import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async (command: string) => {
    if (command === 'get_status') return { state: 'idle' };
    if (command === 'transform_status') return 'idle';
    if (command === 'get_capture_health_history') {
      return {
        schemaVersion: 1,
        observations: Array.from({ length: 5 }, () => ({
          startupMs: 240,
          usedFallback: false,
          fallbackFromBackend: null,
        })),
      };
    }
    throw new Error(`Unexpected command: ${command}`);
  }),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

import { usePerformanceHealth } from './usePerformanceHealth';

function Probe({ enabled }: { enabled: boolean }) {
  const health = usePerformanceHealth(enabled);
  return <output>{health.capture.status}</output>;
}

describe('usePerformanceHealth capture hydration', () => {
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;

  beforeEach(() => {
    mocks.invoke.mockClear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('polls bounded capture history without requesting general event history', async () => {
    await act(async () => {
      root.render(<Probe enabled />);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(mocks.invoke).toHaveBeenCalledWith('get_capture_health_history');
    expect(mocks.invoke).not.toHaveBeenCalledWith('get_event_history');
    expect(container.textContent).toBe('healthy');
  });

  it('does no diagnostic work while disabled', async () => {
    await act(async () => root.render(<Probe enabled={false} />));
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
