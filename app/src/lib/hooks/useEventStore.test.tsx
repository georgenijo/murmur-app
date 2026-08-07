import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(async () => []),
  listen: vi.fn<(event: string, handler: unknown) => void>(),
  unlisten: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: unknown) => {
    mocks.listen(event, handler);
    return mocks.unlisten;
  }),
}));

import { useEventStore } from './useEventStore';

function Probe({ active }: { active: boolean }) {
  const { events } = useEventStore(active);
  return <output>{events.length}</output>;
}

describe('useEventStore activity gating', () => {
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;

  beforeEach(() => {
    mocks.invoke.mockClear();
    mocks.listen.mockClear();
    mocks.unlisten.mockClear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('hydrates and subscribes only while active, then unsubscribes', async () => {
    await act(async () => root.render(<Probe active={false} />));
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.listen).not.toHaveBeenCalled();

    await act(async () => {
      root.render(<Probe active />);
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith('get_event_history');
    expect(mocks.listen).toHaveBeenCalledWith('app-event', expect.any(Function));

    await act(async () => {
      root.render(<Probe active={false} />);
      await Promise.resolve();
    });
    expect(mocks.unlisten).toHaveBeenCalledOnce();
  });
});
