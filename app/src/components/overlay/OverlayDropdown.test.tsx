import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { OverlayDropdown } from './OverlayDropdown';
import type { OverlayGeometry } from '../../lib/overlayGeometry';
import fixture from './overlay-geometry.fixture.json';

it('shows a pending one-recording Mode and restores the bound Mode after consumption', async () => {
  const props = {
    geometry: { dropdownH: 34 } as OverlayGeometry, expanded: true,
    status: 'idle' as const, stillConnecting: false, showTapMissed: false,
    disabled: false, autoPaste: true, fileOutputEnabled: false,
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

it('does not render recording shortcut guidance in the idle quick-controls row', async () => {
  const container = document.createElement('div');
  const root = createRoot(container);
  await act(async () => root.render(
    <OverlayDropdown
      geometry={{ dropdownH: 34 } as OverlayGeometry}
      expanded
      status="idle"
      stillConnecting={false}
      showTapMissed={false}
      disabled={false}
      autoPaste
      fileOutputEnabled={false}
      onToggleDisabled={vi.fn()}
      onToggleAutoPaste={vi.fn()}
      onOpenSettings={vi.fn()}
    />,
  ));

  expect(container.textContent).toBe('Everyday');
  await act(async () => root.unmount());
});

it.each(
  (['notched', 'external', 'fallback'] as const).flatMap((display) =>
    (['recording', 'starting', 'recovering'] as const).map((status) =>
      ({ display, status, geometry: fixture[display] })),
  ),
)(
  'keeps $status status separate from controls ($display)',
  async ({ status, geometry }) => {
    const container = document.createElement('div');
    const root = createRoot(container);
    await act(async () => root.render(
      <OverlayDropdown
        geometry={geometry}
        expanded
        status={status}
        stillConnecting={status === 'starting'}
        showTapMissed={false}
        disabled={false}
        autoPaste
        fileOutputEnabled={false}
        mode={{ id: 'long-mode', name: 'A long Mode name', source: 'manual' }}
        onToggleDisabled={vi.fn()}
        onToggleAutoPaste={vi.fn()}
        onOpenSettings={vi.fn()}
      />,
    ));
    const controls = container.querySelector('[data-testid="overlay-controls-row"]');
    const message = controls?.nextElementSibling;
    expect(controls).not.toBeNull();
    expect(controls?.querySelectorAll('button')).toHaveLength(4);
    expect(message?.parentElement).toBe(controls?.parentElement);
    expect(controls?.contains(message ?? null)).toBe(false);
    expect(message?.getAttribute('role')).toBe(status === 'recording' ? null : 'status');
    expect(message?.textContent).toBe(status === 'recording'
      ? '0:00' : status === 'starting' ? 'Still connecting' : 'Audio recovering');
    await act(async () => root.unmount());
  },
);

it.each([
  ['current', 'Current text field'],
  ['recent', 'Recent text field'],
  ['unavailable', 'Clipboard only'],
] as const)('shows the %s destination in the expanded active overlay', async (smartDestinationStatus, label) => {
  const container = document.createElement('div');
  const root = createRoot(container);
  const props = {
    geometry: fixture.notched,
    expanded: true,
    stillConnecting: false,
    showTapMissed: false,
    smartDestinationStatus,
    disabled: false,
    autoPaste: true,
    fileOutputEnabled: false,
    onToggleDisabled: vi.fn(),
    onToggleAutoPaste: vi.fn(),
    onOpenSettings: vi.fn(),
  };

  await act(async () => root.render(<OverlayDropdown {...props} status="recording" />));
  expect(container.textContent).toContain(`0:00 · ${label}`);

  await act(async () => root.render(<OverlayDropdown {...props} status="processing" />));
  expect(container.textContent).toContain(label);

  await act(async () => root.render(<OverlayDropdown {...props} status="idle" />));
  expect(container.textContent).not.toContain(label);
  await act(async () => root.unmount());
});

it('shows the verified app name for a current or recent overlay destination', async () => {
  const container = document.createElement('div');
  const root = createRoot(container);
  await act(async () => root.render(
    <OverlayDropdown
      geometry={fixture.notched}
      expanded
      status="recording"
      stillConnecting={false}
      showTapMissed={false}
      smartDestinationStatus="recent"
      smartDestinationAppName="T3 Code"
      disabled={false}
      autoPaste
      fileOutputEnabled={false}
      onToggleDisabled={vi.fn()}
      onToggleAutoPaste={vi.fn()}
      onOpenSettings={vi.fn()}
    />,
  ));

  expect(container.textContent).toContain('0:00 · To T3 Code');
  await act(async () => root.unmount());
});
