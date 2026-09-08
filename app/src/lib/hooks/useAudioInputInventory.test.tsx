import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

import { useAudioInputInventory } from './useAudioInputInventory';

const snapshot = (revision: number, name = 'Mic') => ({
  schemaVersion: 2,
  revision,
  status: 'available',
  devices: [{ id: `uid-${revision}`, name, kind: 'external', connected: true, hasInput: true }],
  defaultInputId: `uid-${revision}`,
  lidState: 'open',
  errorCode: null,
});

describe('useAudioInputInventory', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: ReturnType<typeof useAudioInputInventory>;
  let eventHandler: ((event: { payload: unknown }) => void) | null;

  beforeEach(() => {
    eventHandler = null;
    mocks.listen.mockImplementation(async (_name: string, handler: (event: { payload: unknown }) => void) => {
      eventHandler = handler;
      return vi.fn();
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.clearAllMocks();
  });

  async function render(enabled = true) {
    function Harness() {
      current = useAudioInputInventory(enabled);
      return null;
    }
    await act(async () => { root.render(<Harness />); await Promise.resolve(); });
  }

  it('subscribes to the shared event and ignores an older command response', async () => {
    let resolveCommand!: (value: unknown) => void;
    mocks.invoke.mockReturnValue(new Promise((resolve) => { resolveCommand = resolve; }));
    await render();
    await act(async () => eventHandler?.({ payload: snapshot(2, 'New') }));
    await act(async () => resolveCommand(snapshot(1, 'Old')));
    expect(current.inventory?.revision).toBe(2);
    expect(current.inventory?.devices[0].name).toBe('New');
    expect(mocks.listen).toHaveBeenCalledWith('audio-input-inventory-changed', expect.any(Function));
  });

  it('does not let a late command rejection overwrite a newer event', async () => {
    let rejectCommand!: (reason: unknown) => void;
    mocks.invoke.mockReturnValue(new Promise((_resolve, reject) => { rejectCommand = reject; }));
    await render();
    await act(async () => eventHandler?.({ payload: snapshot(2, 'Recovered') }));
    await act(async () => rejectCommand(new Error('old request failed')));
    expect(current.inventory?.devices[0].name).toBe('Recovered');
    expect(current.error).toBeNull();
  });

  it('preserves the last valid snapshot when a later payload is invalid', async () => {
    mocks.invoke.mockResolvedValue(snapshot(1));
    await render();
    await act(async () => eventHandler?.({ payload: { schemaVersion: 1 } }));
    expect(current.inventory?.revision).toBe(1);
    expect(current.error).toContain('unsupported');
  });

  it('retains stale devices for display while reporting refresh unavailability', async () => {
    mocks.invoke.mockResolvedValue(snapshot(1));
    await render();
    await act(async () => eventHandler?.({ payload: {
      ...snapshot(2, 'Cached Mic'),
      status: 'stale',
      errorCode: 'refreshPending',
    } }));
    expect(current.inventory?.status).toBe('stale');
    expect(current.inventory?.devices[0].name).toBe('Cached Mic');
    expect(current.error).toContain('temporarily unavailable');
  });

  it('does not invoke or subscribe while disabled', async () => {
    await render(false);
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.listen).not.toHaveBeenCalled();
  });

  it('fails closed without requesting a snapshot when subscription rejects', async () => {
    mocks.listen.mockRejectedValue(new Error('listener unavailable'));
    mocks.invoke.mockResolvedValue(snapshot(1));
    await render();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(current.inventory).toBeNull();
    expect(current.error).toContain('temporarily unavailable');
  });

  it('waits for listener registration before invoking, closing the missed-event window', async () => {
    let resolveListen!: (stop: () => void) => void;
    let resolveCommand!: (value: unknown) => void;
    const stop = vi.fn();
    mocks.listen.mockImplementation((_name: string, handler: (event: { payload: unknown }) => void) => {
      eventHandler = handler;
      return new Promise((resolve) => { resolveListen = resolve; });
    });
    mocks.invoke.mockReturnValue(new Promise((resolve) => { resolveCommand = resolve; }));
    await render();
    expect(mocks.invoke).not.toHaveBeenCalled();
    await act(async () => { resolveListen(stop); await Promise.resolve(); });
    expect(mocks.invoke).toHaveBeenCalledWith('get_audio_input_inventory');
    await act(async () => eventHandler?.({ payload: snapshot(2, 'Event after registration') }));
    await act(async () => resolveCommand(snapshot(1, 'Older command')));
    expect(current.inventory?.revision).toBe(2);
    expect(current.inventory?.devices[0].name).toBe('Event after registration');
  });

  it('cleans up a subscription that resolves after disable without invoking', async () => {
    let resolveListen!: (stop: () => void) => void;
    const stop = vi.fn();
    mocks.listen.mockReturnValue(new Promise((resolve) => { resolveListen = resolve; }));
    function Harness({ enabled }: { enabled: boolean }) {
      current = useAudioInputInventory(enabled);
      return null;
    }
    await act(async () => { root.render(<Harness enabled />); await Promise.resolve(); });
    await act(async () => { root.render(<Harness enabled={false} />); await Promise.resolve(); });
    await act(async () => {
      resolveListen(stop);
      await Promise.resolve();
    });
    expect(stop).toHaveBeenCalledOnce();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(current.inventory).toBeNull();
  });

  it('invalidates a command response that resolves after disable', async () => {
    let resolveCommand!: (value: unknown) => void;
    mocks.invoke.mockReturnValue(new Promise((resolve) => { resolveCommand = resolve; }));
    function Harness({ enabled }: { enabled: boolean }) {
      current = useAudioInputInventory(enabled);
      return null;
    }
    await act(async () => { root.render(<Harness enabled />); await Promise.resolve(); });
    await act(async () => { root.render(<Harness enabled={false} />); await Promise.resolve(); });
    await act(async () => { resolveCommand(snapshot(9)); await Promise.resolve(); });
    expect(current.inventory).toBeNull();
  });
});
