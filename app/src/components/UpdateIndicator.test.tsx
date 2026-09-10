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
    const onDownload = vi.fn();
    await act(async () => {
      root.render(
        <UpdateIndicator
          status={{ phase: 'available', version: 'v0.23.0', notes: '', isForced: false }}
          onOpen={vi.fn()}
          onDownload={onDownload}
          onRestart={vi.fn()}
          onRetryCheck={vi.fn()}
        />,
      );
    });

    const button = container.querySelector('button');
    expect(button?.getAttribute('aria-label')).toBe('Download Update. Murmur v0.23.0 is available');
    expect(button?.textContent).toBe('');
    await act(async () => button?.click());
    expect(onDownload).toHaveBeenCalledOnce();
  });

  it('reports manual check progress and success without creating a dead button', async () => {
    await act(async () => {
      root.render(
        <UpdateIndicator
          status={{ phase: 'checking' }}
          onOpen={vi.fn()}
          onDownload={vi.fn()}
          onRestart={vi.fn()}
          onRetryCheck={vi.fn()}
        />,
      );
    });
    expect(container.querySelector('[role="status"]')?.textContent).toBe('Checking for Updates…');

    await act(async () => {
      root.render(
        <UpdateIndicator
          status={{ phase: 'up-to-date' }}
          onOpen={vi.fn()}
          onDownload={vi.fn()}
          onRestart={vi.fn()}
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
          status={{ phase: 'error', stage: 'check', message: 'offline', isForced: false }}
          onOpen={vi.fn()}
          onDownload={vi.fn()}
          onRestart={vi.fn()}
          onRetryCheck={onRetryCheck}
        />,
      );
    });

    await act(async () => container.querySelector('button')?.click());
    expect(onRetryCheck).toHaveBeenCalledOnce();
  });

  it('reopens installation guidance instead of retrying update discovery', async () => {
    const onOpen = vi.fn();
    const onRetryCheck = vi.fn();
    await act(async () => {
      root.render(
        <UpdateIndicator
          status={{
            phase: 'error',
            stage: 'install',
            message: 'Move Murmur to Applications',
            isForced: false,
            recovery: 'reinstall',
          }}
          onOpen={onOpen}
          onDownload={vi.fn()}
          onRestart={vi.fn()}
          onRetryCheck={onRetryCheck}
        />,
      );
    });

    expect(container.querySelector('button')?.getAttribute('aria-label')).toBe('Update Needs Attention. Open the update dialog to continue.');
    await act(async () => container.querySelector('button')?.click());
    expect(onOpen).toHaveBeenCalledOnce();
    expect(onRetryCheck).not.toHaveBeenCalled();
  });

  it('stays absent while idle', async () => {
    await act(async () => {
      root.render(
        <UpdateIndicator
          status={{ phase: 'idle' }}
          onOpen={vi.fn()}
          onDownload={vi.fn()}
          onRestart={vi.fn()}
          onRetryCheck={vi.fn()}
        />,
      );
    });
    expect(container.innerHTML).toBe('');
  });
});
