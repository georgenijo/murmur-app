import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DiscoveryHintToast } from './DiscoveryHintToast';

describe('discovery UI', () => {
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

  it('announces each actionable hint and gives dismissal a specific label', async () => {
    const onAction = vi.fn();
    const onDismiss = vi.fn();
    await act(async () => root.render(
      <DiscoveryHintToast hint="double_tap_timing" onAction={onAction} onDismiss={onDismiss} />,
    ));
    expect(container.querySelector('[aria-live="polite"]')).not.toBeNull();
    await act(async () => (container.querySelector('[aria-label^="Dismiss A double-tap"]') as HTMLButtonElement).click());
    expect(onDismiss).toHaveBeenCalledWith('double_tap_timing');
  });
});
