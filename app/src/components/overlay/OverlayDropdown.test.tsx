import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { OverlayDropdown } from './OverlayDropdown';
import type { OverlayGeometry } from '../../lib/overlayGeometry';
import fixture from './overlay-geometry.fixture.json';

it('offers only the disable, auto-paste, and settings quick controls', async () => {
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
  const labels = [...container.querySelectorAll('button')].map((button) => button.getAttribute('aria-label'));
  expect(labels).toHaveLength(3);
  expect(labels.join(' ')).not.toMatch(/mode/i);
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

  expect(container.textContent).toBe('');
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
        onToggleDisabled={vi.fn()}
        onToggleAutoPaste={vi.fn()}
        onOpenSettings={vi.fn()}
      />,
    ));
    const controls = container.querySelector('[data-testid="overlay-controls-row"]');
    const message = controls?.nextElementSibling;
    expect(controls).not.toBeNull();
    expect(controls?.querySelectorAll('button')).toHaveLength(3);
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

it('grows the floating pill controls upward as one segmented strip', async () => {
  const onToggleAutoPaste = vi.fn();
  const container = document.createElement('div');
  const root = createRoot(container);
  const props = {
    geometry: fixture.pillExpanded as OverlayGeometry, expanded: true,
    status: 'idle' as const, stillConnecting: false, showTapMissed: false,
    disabled: false, fileOutputEnabled: false,
    onToggleDisabled: vi.fn(), onToggleAutoPaste, onOpenSettings: vi.fn(),
  };
  await act(async () => root.render(<OverlayDropdown {...props} autoPaste />));
  const strip = container.querySelector<HTMLElement>('[role="group"][aria-label="Quick settings"]')!;
  expect(strip.classList.contains('bezel-strip')).toBe(true);
  expect(strip.querySelectorAll('[data-testid="overlay-controls-row"] .bezel-segment')).toHaveLength(3);
  const paste = container.querySelector<HTMLButtonElement>('[role="switch"]')!;
  expect(paste.getAttribute('aria-checked')).toBe('true');
  expect(paste.textContent).toBe('Paste');
  await act(async () => paste.click());
  expect(onToggleAutoPaste).toHaveBeenCalledOnce();
  await act(async () => root.render(<OverlayDropdown {...props} autoPaste={false} />));
  expect(paste.getAttribute('aria-checked')).toBe('false');
  await act(async () => root.unmount());
});

it('keeps the floating strip monochrome in every toggle state', async () => {
  const container = document.createElement('div');
  const root = createRoot(container);
  const props = {
    geometry: fixture.pillExpanded as OverlayGeometry, expanded: true,
    status: 'idle' as const, stillConnecting: false, showTapMissed: false, fileOutputEnabled: false,
    onToggleDisabled: vi.fn(), onToggleAutoPaste: vi.fn(), onOpenSettings: vi.fn(),
  };
  for (const [disabled, autoPaste] of [[false, true], [true, true], [false, false], [true, false]]) {
    await act(async () => root.render(<OverlayDropdown {...props} disabled={disabled} autoPaste={autoPaste} />));
    const strokes = [...container.querySelectorAll('[data-testid="overlay-controls-row"] [stroke]')]
      .map(el => el.getAttribute('stroke'));
    expect(strokes.length).toBeGreaterThan(0);
    expect(new Set(strokes)).toEqual(new Set(['currentColor']));
  }
  // State reads from the label and the selected background, not a hue.
  expect(container.querySelector('[aria-label="Enable Murmur"]')?.textContent).toBe('Off');
  expect(container.querySelector('[role="switch"]')?.getAttribute('aria-checked')).toBe('false');
  await act(async () => root.unmount());
});
