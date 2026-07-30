import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { UpdateIndicator } from './UpdateIndicator';

describe('UpdateIndicator', () => {
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

  it('keeps an available update actionable after passive discovery', async () => {
    const onOpen = vi.fn();
    await act(async () => {
      root.render(
        <UpdateIndicator
          status={{ phase: 'available', version: 'v0.23.0', notes: '', isForced: false }}
          onOpen={onOpen}
          onRetryCheck={vi.fn()}
        />,
      );
    });

    expect(container.textContent).toContain('Update available · v0.23.0');
    await act(async () => container.querySelector('button')?.click());
    expect(onOpen).toHaveBeenCalledOnce();
  });

  it('reports manual check progress and success without creating a dead button', async () => {
    await act(async () => {
      root.render(
        <UpdateIndicator
          status={{ phase: 'checking' }}
          onOpen={vi.fn()}
          onRetryCheck={vi.fn()}
        />,
      );
    });
    expect(container.querySelector('[role="status"]')?.textContent).toContain('Checking');

    await act(async () => {
      root.render(
        <UpdateIndicator
          status={{ phase: 'up-to-date' }}
          onOpen={vi.fn()}
          onRetryCheck={vi.fn()}
        />,
      );
    });
    expect(container.querySelector('[role="status"]')?.textContent).toContain('up to date');
  });

  it('lets a failed check retry the check itself', async () => {
    const onRetryCheck = vi.fn();
    await act(async () => {
      root.render(
        <UpdateIndicator
          status={{ phase: 'error', message: 'offline', isForced: false }}
          onOpen={vi.fn()}
          onRetryCheck={onRetryCheck}
        />,
      );
    });

    await act(async () => container.querySelector('button')?.click());
    expect(onRetryCheck).toHaveBeenCalledOnce();
  });

  it('stays absent while idle', async () => {
    await act(async () => {
      root.render(
        <UpdateIndicator
          status={{ phase: 'idle' }}
          onOpen={vi.fn()}
          onRetryCheck={vi.fn()}
        />,
      );
    });
    expect(container.innerHTML).toBe('');
  });
});
