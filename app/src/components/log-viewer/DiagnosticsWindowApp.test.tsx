import { readFileSync } from 'node:fs';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const listeners = vi.hoisted(() => new Map<string, (event: { payload: unknown }) => void>());

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(name, handler);
    return () => { listeners.delete(name); };
  }),
}));

const windowState = vi.hoisted(() => ({ tauri: false, visible: false }));
const invokeMock = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => windowState.tauri,
  invoke: invokeMock,
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ isVisible: async () => windowState.visible }),
}));

const workspaceMock = vi.hoisted(() => vi.fn());

vi.mock('./DiagnosticsWorkspace', () => ({
  DiagnosticsWorkspace: (props: unknown) => {
    workspaceMock(props);
    return <div>Diagnostics workspace</div>;
  },
  isDiagnosticsTab: (value: string) => ['events', 'runs', 'performance'].includes(value),
}));

import { DiagnosticsWindowApp } from './DiagnosticsWindowApp';

function lastProps(): { active: boolean; requestedTab: string } {
  return workspaceMock.mock.calls[workspaceMock.mock.calls.length - 1]?.[0] as { active: boolean; requestedTab: string };
}

describe('DiagnosticsWindowApp native chrome', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    workspaceMock.mockClear();
    listeners.clear();
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
    windowState.tauri = false;
    windowState.visible = false;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<DiagnosticsWindowApp />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('pairs drag-marked title bars with permissions for both movable windows', () => {
    expect(container.querySelector('header[data-tauri-drag-region]')).not.toBeNull();

    const diagnosticsCapability = JSON.parse(
      readFileSync('./src-tauri/capabilities/diagnostics.json', 'utf8'),
    ) as { permissions: string[] };
    const mainCapability = JSON.parse(
      readFileSync('./src-tauri/capabilities/default.json', 'utf8'),
    ) as { permissions: string[] };

    expect(diagnosticsCapability.permissions).toContain('core:window:allow-start-dragging');
    expect(mainCapability.permissions).toContain('core:window:allow-start-dragging');
  });

  it('enables requester-gated diagnostics store health in this webview', () => {
    expect(workspaceMock).toHaveBeenCalledWith(expect.objectContaining({
      storeHealthEnabled: true,
    }));
  });

  it('keeps the workspace inactive while the window is hidden at launch', () => {
    expect(workspaceMock).toHaveBeenCalled();
    for (const [props] of workspaceMock.mock.calls) {
      expect((props as { active: boolean }).active).toBe(false);
    }
  });

  it('activates the workspace and selects the tab when the window is shown', async () => {
    await act(async () => {
      listeners.get('diagnostics-tab-requested')?.({ payload: { tab: 'runs', seq: 1 } });
    });
    expect(lastProps()).toMatchObject({ active: true, requestedTab: 'runs' });
  });

  it('activates the workspace when the window gains focus', async () => {
    expect(lastProps().active).toBe(false);
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
    });
    expect(lastProps().active).toBe(true);
  });

  it('stays active once shown, so reopening does not reload the workspace', async () => {
    await act(async () => {
      listeners.get('diagnostics-tab-requested')?.({ payload: { tab: 'runs', seq: 1 } });
    });
    await act(async () => {
      listeners.get('diagnostics-tab-requested')?.({ payload: { tab: 'performance', seq: 2 } });
    });
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
    });
    for (const [props] of workspaceMock.mock.calls.slice(workspaceMock.mock.calls.findIndex(([p]) => (p as { active: boolean }).active))) {
      expect((props as { active: boolean }).active).toBe(true);
    }
    expect(lastProps()).toMatchObject({ active: true, requestedTab: 'performance' });
  });

  it('activates on mount when the window was already visible before the listeners attached', async () => {
    await act(async () => root.unmount());
    workspaceMock.mockClear();
    windowState.tauri = true;
    windowState.visible = true;
    root = createRoot(container);
    await act(async () => root.render(<DiagnosticsWindowApp />));
    expect(lastProps().active).toBe(true);
  });

  it('stays inactive on mount when the native window is still hidden', async () => {
    await act(async () => root.unmount());
    workspaceMock.mockClear();
    windowState.tauri = true;
    windowState.visible = false;
    root = createRoot(container);
    await act(async () => root.render(<DiagnosticsWindowApp />));
    expect(lastProps().active).toBe(false);
  });

  async function mountInTauri() {
    await act(async () => root.unmount());
    workspaceMock.mockClear();
    windowState.tauri = true;
    root = createRoot(container);
  }

  it('reads the retained request after subscribing, so a request made during startup is not lost', async () => {
    await mountInTauri();
    invokeMock.mockResolvedValue({ tab: 'runs', seq: 1 });
    await act(async () => root.render(<DiagnosticsWindowApp />));
    expect(invokeMock).toHaveBeenCalledWith('get_requested_diagnostics_tab');
    expect(listeners.has('diagnostics-tab-requested')).toBe(true);
    expect(lastProps()).toMatchObject({ active: true, requestedTab: 'runs' });
  });

  it('keeps the newer of two different requests when the read answers after the event', async () => {
    await mountInTauri();
    let answer: (value: unknown) => void = () => {};
    invokeMock.mockReturnValue(new Promise(resolve => { answer = resolve; }));
    await act(async () => root.render(<DiagnosticsWindowApp />));
    // Request 2 (Performance) is delivered while the read of request 1 (Runs) is in flight.
    await act(async () => {
      listeners.get('diagnostics-tab-requested')?.({ payload: { tab: 'performance', seq: 2 } });
    });
    await act(async () => answer({ tab: 'runs', seq: 1 }));
    expect(lastProps()).toMatchObject({ active: true, requestedTab: 'performance' });
  });

  it('applies a newer read over an older event', async () => {
    await mountInTauri();
    let answer: (value: unknown) => void = () => {};
    invokeMock.mockReturnValue(new Promise(resolve => { answer = resolve; }));
    await act(async () => root.render(<DiagnosticsWindowApp />));
    await act(async () => {
      listeners.get('diagnostics-tab-requested')?.({ payload: { tab: 'events', seq: 1 } });
    });
    await act(async () => answer({ tab: 'runs', seq: 2 }));
    expect(lastProps()).toMatchObject({ active: true, requestedTab: 'runs' });
  });

  it('ignores an unknown tab and malformed requests', async () => {
    await mountInTauri();
    invokeMock.mockResolvedValue({ tab: 'nope', seq: 1 });
    await act(async () => root.render(<DiagnosticsWindowApp />));
    await act(async () => {
      listeners.get('diagnostics-tab-requested')?.({ payload: 'runs' });
    });
    expect(lastProps()).toMatchObject({ requestedTab: 'events' });
  });

  it('activates on mount when the document already has focus', async () => {
    await act(async () => root.unmount());
    workspaceMock.mockClear();
    const focus = vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    root = createRoot(container);
    await act(async () => root.render(<DiagnosticsWindowApp />));
    expect(lastProps().active).toBe(true);
    focus.mockRestore();
  });
});
