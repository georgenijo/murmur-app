import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { OverlayDropdown } from './OverlayDropdown';
import type { OverlayGeometry } from '../../lib/overlayGeometry';

it('shows a pending one-recording Mode and restores the bound Mode after consumption', async () => {
  const props = {
    geometry: { dropdownH: 34 } as OverlayGeometry, expanded: true,
    status: 'idle' as const, stillConnecting: false, showTapMissed: false,
    disabled: false, autoPaste: true, fileOutputEnabled: false, recordingShortcutHint: 'Hold Fn',
    onToggleDisabled: vi.fn(), onToggleAutoPaste: vi.fn(), onOpenSettings: vi.fn(),
  };
  const mode = { id: 'builtin.email', name: 'Email', source: 'app_binding' as const };
  const container = document.createElement('div');
  const root = createRoot(container);
  await act(async () => root.render(<OverlayDropdown {...props} mode={{ ...mode, pending: { id: 'builtin.verbatim', name: 'Verbatim' } }} />));
  expect(container.querySelector('[aria-label="Next recording: Verbatim. Current Mode: Email"]')?.textContent).toBe('Next: Verbatim');
  await act(async () => root.render(<OverlayDropdown {...props} mode={{ ...mode, pending: null }} />));
  expect(container.textContent).not.toContain('Next: Verbatim');
  expect(container.querySelector('[aria-label="Mode: Email. Click to cycle"]')?.textContent).toBe('Email');
  await act(async () => root.unmount());
});
