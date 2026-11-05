import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DiscoveryHintToast } from './DiscoveryHintToast';
import { DiscoveryChecklist } from './home/DiscoveryChecklist';

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

  it('renders five labelled actions and updates its accessible completion count', async () => {
    const onAction = vi.fn();
    await act(async () => root.render(
      <DiscoveryChecklist completed={[]} onAction={onAction} onDismiss={vi.fn()} />,
    ));
    expect(container.querySelectorAll('.discovery-checklist-items li')).toHaveLength(5);
    expect(container.querySelector('[aria-label="0 of 5 complete"]')).not.toBeNull();
    expect(container.querySelector('section')?.getAttribute('aria-labelledby')).toBe('discovery-checklist-title');
    const transform = Array.from(container.querySelectorAll('button'))
      .find((button) => button.textContent === 'Open Notetaker') as HTMLButtonElement;
    await act(async () => transform.click());
    expect(onAction).toHaveBeenCalledWith('meeting');

    await act(async () => root.render(
      <DiscoveryChecklist completed={['meeting']} onAction={onAction} onDismiss={vi.fn()} />,
    ));
    expect(container.querySelector('[aria-label="1 of 5 complete"]')).not.toBeNull();
    expect(container.querySelector('[data-complete="true"]')?.textContent).toContain('Open again');
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
