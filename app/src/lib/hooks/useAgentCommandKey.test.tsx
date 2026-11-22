import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentCommandKey } from '../settings';

type Listener = (event: { payload: unknown }) => void;

const mocks = vi.hoisted(() => ({
  setAgentCommandShortcut: vi.fn(async (_shortcut: string | null) => undefined),
  listeners: new Map<string, Listener>(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, listener: Listener) => {
    mocks.listeners.set(event, listener);
    return () => mocks.listeners.delete(event);
  }),
}));
vi.mock('../dictation', () => ({ setAgentCommandShortcut: mocks.setAgentCommandShortcut }));

import { useAgentCommandKey } from './useAgentCommandKey';

const onStart = vi.fn();
const onStop = vi.fn();
const onCancel = vi.fn();

// One component type for every render, so a setting change updates the mounted
// hook instead of remounting it.
function Harness({ enabled, shortcut }: { enabled: boolean; shortcut: AgentCommandKey }) {
  useAgentCommandKey({ enabled, shortcut, onStart, onStop, onCancel });
  return null;
}

describe('useAgentCommandKey', () => {
  let container: HTMLDivElement;
  let root: Root;

  async function render(enabled: boolean, shortcut: AgentCommandKey = 'alt_r') {
    await act(async () => {
      root.render(<Harness enabled={enabled} shortcut={shortcut} />);
      await Promise.resolve();
    });
  }

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('binds nothing and listens to nothing while the setting is off', async () => {
    await render(false);
    expect(mocks.setAgentCommandShortcut).not.toHaveBeenCalled();
    expect(mocks.listeners.size).toBe(0);
  });

  it('binds the key once listening, and forwards the hold events', async () => {
    await render(true);
    expect(mocks.setAgentCommandShortcut).toHaveBeenCalledExactlyOnceWith('alt_r');
    expect([...mocks.listeners.keys()].sort()).toEqual([
      'agent-command-cancel', 'agent-command-start', 'agent-command-stop',
    ]);

    mocks.listeners.get('agent-command-start')!({ payload: null });
    mocks.listeners.get('agent-command-stop')!({ payload: null });
    mocks.listeners.get('agent-command-cancel')!({ payload: null });
    expect(onStart).toHaveBeenCalledOnce();
    expect(onStop).toHaveBeenCalledOnce();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it('unbinds the key and discards a hold in progress when the setting turns off', async () => {
    await render(true);
    mocks.setAgentCommandShortcut.mockClear();
    expect(onCancel).not.toHaveBeenCalled();
    await render(false);
    expect(mocks.setAgentCommandShortcut).toHaveBeenCalledExactlyOnceWith(null);
    expect(mocks.listeners.size).toBe(0);
    // The release can no longer be heard, so the recording is not left running.
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it('rebinds when the key changes', async () => {
    await render(true, 'alt_r');
    await render(true, 'shift_r');
    expect(mocks.setAgentCommandShortcut.mock.calls.map(([shortcut]) => shortcut)).toEqual([
      'alt_r', null, 'shift_r',
    ]);
  });

  it('keeps the app running when the key cannot be bound', async () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.setAgentCommandShortcut.mockRejectedValueOnce(new Error('Accessibility permission is required'));
    await render(true);
    expect(consoleError).toHaveBeenCalled();
    consoleError.mockRestore();
  });
});
