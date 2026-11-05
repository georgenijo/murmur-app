import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { DiscoveryChecklist } from './DiscoveryChecklist';

it('renders complete source-owned setup help and delivers setup/dismiss actions separately', async () => {
  const container = document.createElement('div');
  const root = createRoot(container);
  const onAction = vi.fn();
  const onDismiss = vi.fn();
  try {
    await act(async () => root.render(<DiscoveryChecklist completed={['mode_binding']} onAction={onAction} onDismiss={onDismiss} />));
    const items = [...container.querySelectorAll('li')];
    expect(items).toHaveLength(5);
    expect(items[1].querySelector('strong')?.textContent).toBe('Bind a mode to an app');
    expect(items[1].querySelector('small')?.textContent).toBe('Give one app its own writing behavior');
    expect(items[1].querySelector('button')?.textContent).toBe('Open again');
    await act(async () => items[1].querySelector<HTMLButtonElement>('button')!.click());
    expect(onAction).toHaveBeenCalledExactlyOnceWith('mode_binding');
    expect(onDismiss).not.toHaveBeenCalled();
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Dismiss Get more from Murmur"]')!.click());
    expect(onDismiss).toHaveBeenCalledOnce();
    expect(onAction).toHaveBeenCalledOnce();
  } finally {
    await act(async () => root.unmount());
  }
});
