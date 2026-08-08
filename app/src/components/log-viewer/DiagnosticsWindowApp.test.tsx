import { readFileSync } from 'node:fs';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock('./DiagnosticsWorkspace', () => ({
  DiagnosticsWorkspace: () => <div>Diagnostics workspace</div>,
  isDiagnosticsTab: () => true,
}));

import { DiagnosticsWindowApp } from './DiagnosticsWindowApp';

describe('DiagnosticsWindowApp native chrome', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
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
});
