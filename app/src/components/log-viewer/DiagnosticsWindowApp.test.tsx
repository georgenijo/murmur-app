import { readFileSync } from 'node:fs';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}));

const workspaceMock = vi.hoisted(() => vi.fn());
const visibility = vi.hoisted(() => ({ visible: false }));

vi.mock('../../lib/hooks/useNativeWindowVisible', () => ({
  useNativeWindowVisible: () => visibility.visible,
}));

vi.mock('./DiagnosticsWorkspace', () => ({
  DiagnosticsWorkspace: (props: unknown) => {
    workspaceMock(props);
    return <div>Diagnostics workspace</div>;
  },
  isDiagnosticsTab: () => true,
}));

import { DiagnosticsWindowApp } from './DiagnosticsWindowApp';

describe('DiagnosticsWindowApp native chrome', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    visibility.visible = false;
    workspaceMock.mockClear();
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

  it('keeps a hidden native window inactive and resumes its retained workspace on reopen', async () => {
    expect(workspaceMock).toHaveBeenLastCalledWith(expect.objectContaining({ active: false }));
    visibility.visible = true;
    await act(async () => root.render(<DiagnosticsWindowApp />));
    expect(workspaceMock).toHaveBeenLastCalledWith(expect.objectContaining({ active: true }));
    visibility.visible = false;
    await act(async () => root.render(<DiagnosticsWindowApp />));
    expect(workspaceMock).toHaveBeenLastCalledWith(expect.objectContaining({ active: false }));
  });
});
