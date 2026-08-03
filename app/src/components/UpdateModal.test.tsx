import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { UpdateModal } from './UpdateModal';

describe('UpdateModal', () => {
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

  it('makes environment verification visibly busy and non-actionable', async () => {
    const onDownload = vi.fn();
    const onDismiss = vi.fn();
    await act(async () => {
      root.render(
        <UpdateModal
          status={{ phase: 'preparing', version: '0.24.3' }}
          onDownload={onDownload}
          onSkip={vi.fn()}
          onDismiss={onDismiss}
        />,
      );
    });

    expect(container.textContent).toContain('Preparing update...');
    expect(container.querySelector('button')).toBeNull();

    await act(async () => {
      (container.firstElementChild as HTMLElement).click();
    });
    expect(onDownload).not.toHaveBeenCalled();
    expect(onDismiss).not.toHaveBeenCalled();
  });
});
