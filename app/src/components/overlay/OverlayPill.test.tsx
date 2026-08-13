import { act, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { OverlayGeometry } from '../../lib/overlayGeometry';
import { OverlayPill } from './OverlayPill';

const geometry: OverlayGeometry = {
  windowW: 257,
  collapsedH: 32,
  expandedH: 76,
  pillIdleW: 221,
  pillActiveW: 257,
  pillMarginIdle: 0,
  pillMarginActive: 0,
  dropdownH: 44,
  wingW: 36,
};

function ClipboardOnlyPill() {
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);
  return (
    <OverlayPill
      geometry={geometry}
      visual={{
        indicator: { kind: 'clipboardOnly' },
        showTapMissedLabel: false,
        waveformVisible: false,
      }}
      status="idle"
      barRefs={barRefs}
    />
  );
}

describe('OverlayPill clipboard-only cue', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('renders an accessible, non-interactive manual-paste status', async () => {
    await act(async () => root.render(<ClipboardOnlyPill />));

    const status = container.querySelector<HTMLElement>('[role="status"]');
    expect(status?.textContent).toBe('⌘V');
    expect(status?.getAttribute('aria-live')).toBe('polite');
    expect(status?.getAttribute('aria-label')).toBe('Text copied to clipboard. Paste manually.');
    expect(container.querySelector('button')).toBeNull();
  });
});
