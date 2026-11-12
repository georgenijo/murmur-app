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
      listeners.get('diagnostics-tab-requested')?.({ payload: 'runs' });
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
      listeners.get('diagnostics-tab-requested')?.({ payload: 'runs' });
    });
    await act(async () => {
      listeners.get('diagnostics-tab-requested')?.({ payload: 'performance' });
    });
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
    });
    for (const [props] of workspaceMock.mock.calls.slice(workspaceMock.mock.calls.findIndex(([p]) => (p as { active: boolean }).active))) {
      expect((props as { active: boolean }).active).toBe(true);
    }
    expect(lastProps()).toMatchObject({ active: true, requestedTab: 'performance' });
  });
});
