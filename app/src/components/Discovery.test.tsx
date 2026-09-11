import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DiscoveryHintToast } from './DiscoveryHintToast';
import { DiscoveryChecklist } from './home/DiscoveryChecklist';
import { TransformPractice } from './settings/TransformPractice';

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

  it('renders seven labelled actions and updates its accessible completion count', async () => {
    const onAction = vi.fn();
    await act(async () => root.render(
      <DiscoveryChecklist completed={[]} onAction={onAction} onDismiss={vi.fn()} />,
    ));
    expect(container.querySelectorAll('.discovery-checklist-items li')).toHaveLength(7);
    expect(container.querySelector('[aria-label="0 of 7 complete"]')).not.toBeNull();
    expect(container.querySelector('section')?.getAttribute('aria-labelledby')).toBe('discovery-checklist-title');
    const transform = Array.from(container.querySelectorAll('button'))
      .find((button) => button.textContent === 'Try it') as HTMLButtonElement;
    await act(async () => transform.click());
    expect(onAction).toHaveBeenCalledWith('transform');

    await act(async () => root.render(
      <DiscoveryChecklist completed={['transform']} onAction={onAction} onDismiss={vi.fn()} />,
    ));
    expect(container.querySelector('[aria-label="1 of 7 complete"]')).not.toBeNull();
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

  it('selects only synthetic practice text and explains missing setup', async () => {
    await act(async () => root.render(<TransformPractice transformHoldKey={null} modelReady />));
    expect((container.querySelector('button') as HTMLButtonElement).disabled).toBe(true);
    expect(container.textContent).toContain('Enable the Transform shortcut above first.');

    await act(async () => root.render(<TransformPractice transformHoldKey="alt_r" modelReady />));
    const textarea = container.querySelector('textarea') as HTMLTextAreaElement;
    expect(textarea.readOnly).toBe(false);
    await act(async () => (container.querySelector('button') as HTMLButtonElement).click());
    expect(document.activeElement).toBe(textarea);
    expect(textarea.selectionStart).toBe(0);
    expect(textarea.selectionEnd).toBe(textarea.value.length);
    expect(container.textContent).toContain('say “make this shorter,” then release');

    textarea.value = 'A native-approved rewrite stays here.';
    await act(async () => root.render(<TransformPractice transformHoldKey="alt_r" modelReady />));
    expect((container.querySelector('textarea') as HTMLTextAreaElement).value)
      .toBe('A native-approved rewrite stays here.');
  });
});
