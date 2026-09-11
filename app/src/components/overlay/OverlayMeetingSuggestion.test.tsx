import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { OverlayGeometry } from '../../lib/overlayGeometry';
import { OverlayMeetingSuggestion } from './OverlayMeetingSuggestion';

const geometry: OverlayGeometry = {
  windowW: 320,
  collapsedH: 32,
  expandedH: 168,
  pillIdleW: 221,
  pillActiveW: 320,
  pillMarginIdle: 31.5,
  pillMarginActive: 0,
  dropdownH: 136,
  wingW: 36,
};

describe('OverlayMeetingSuggestion', () => {
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

  it('shows the prompt with keyboard-native Accept and Dismiss buttons', async () => {
    const onAccept = vi.fn();
    const onDismiss = vi.fn();
    await act(async () => root.render(
      <OverlayMeetingSuggestion
        geometry={geometry}
        expanded
        suggestion={{ token: '55555555-5555-4555-8555-555555555555', title: 'Product review', startMs: 1, endMs: 2 }}
        busy={false}
        error={null}
        onAccept={onAccept}
        onDismiss={onDismiss}
      />,
    ));

    const buttons = Array.from(container.querySelectorAll('button'));
    expect(buttons.map((button) => button.textContent?.trim())).toEqual(['Accept', 'Dismiss']);
    expect(buttons.every((button) => button.type === 'button')).toBe(true);
    await act(async () => buttons[0].click());
    await act(async () => buttons[1].click());
    expect(onAccept).toHaveBeenCalledOnce();
    expect(onDismiss).toHaveBeenCalledOnce();
    expect(container.textContent).toContain('Product review started. Start Notetaker?');
  });
});
