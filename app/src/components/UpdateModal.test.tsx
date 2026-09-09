import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  exit: vi.fn(),
  openUrl: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-process', () => ({ exit: mocks.exit }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: mocks.openUrl }));

import { LATEST_RELEASES_URL, UpdateModal } from './UpdateModal';

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
          onRetryCheck={vi.fn()}
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

  it('offers release-page recovery and retries the check after a check failure', async () => {
    const onRetryCheck = vi.fn();
    await act(async () => {
      root.render(
        <UpdateModal
          status={{ phase: 'error', stage: 'check', message: 'offline', isForced: false }}
          onDownload={vi.fn()}
          onRetryCheck={onRetryCheck}
          onSkip={vi.fn()}
          onDismiss={vi.fn()}
        />,
      );
    });

    const buttons = Array.from(container.querySelectorAll('button'));
    expect(buttons.map((button) => button.textContent)).toContain('Retry');
    expect(buttons.map((button) => button.textContent)).toContain('Download latest version');

    await act(async () => buttons.find((button) => button.textContent === 'Retry')?.click());
    expect(onRetryCheck).toHaveBeenCalledOnce();

    await act(async () => buttons.find((button) => button.textContent === 'Download latest version')?.click());
    expect(mocks.openUrl).toHaveBeenCalledWith(LATEST_RELEASES_URL);
  });

  it('gives release notes the flexible reading area and keeps secondary actions together', async () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    await act(async () => {
      root.render(
        <UpdateModal
          status={{
            phase: 'available',
            version: '0.43.1',
            notes: '## New Features\n\n- A useful change.\n- Another useful change.',
            isForced: false,
          }}
          onDownload={vi.fn()}
          onRetryCheck={vi.fn()}
          onSkip={vi.fn()}
          onDismiss={onDismiss}
        />,
      );
    });
    await act(async () => vi.advanceTimersByTime(60));

    const dialog = container.querySelector('[role="dialog"]') as HTMLDivElement;
    const notes = dialog.querySelector('.flex-1.overflow-y-auto');
    const skip = Array.from(dialog.querySelectorAll('button')).find((button) =>
      button.textContent === 'Skip This Version');
    const secondaryActions = skip?.parentElement;

    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.className).toContain('max-w-[560px]');
    expect(dialog.className).toContain('h-[min(620px,calc(100vh-2.5rem))]');
    expect(notes?.textContent).toContain('A useful change.');
    expect(secondaryActions?.classList.contains('grid-cols-2')).toBe(true);
    expect(document.activeElement?.textContent).toBe('Update Now');

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(onDismiss).toHaveBeenCalledOnce();
    vi.useRealTimers();
  });

  it('keeps short progress states compact', async () => {
    await act(async () => {
      root.render(
        <UpdateModal
          status={{ phase: 'downloading', version: '0.43.1', progress: 50 }}
          onDownload={vi.fn()}
          onRetryCheck={vi.fn()}
          onSkip={vi.fn()}
          onDismiss={vi.fn()}
        />,
      );
    });

    const dialog = container.querySelector('[role="dialog"]') as HTMLDivElement;
    expect(dialog.className).toContain('max-h-[calc(100vh-2.5rem)]');
    expect(dialog.className).not.toContain('h-[min(620px,calc(100vh-2.5rem))]');
  });

  it.each([
    ['idle', { phase: 'idle' }],
    ['checking', { phase: 'checking' }],
    ['up-to-date', { phase: 'up-to-date' }],
    ['available', { phase: 'available', version: '0.24.3', notes: '', isForced: false }],
    ['downloading', { phase: 'downloading', version: '0.24.3', progress: 50 }],
    ['ready', { phase: 'ready', version: '0.24.3' }],
    ['install error', { phase: 'error', stage: 'install', message: 'disk full', isForced: false }],
  ] as const)('does not show release-page recovery in %s state', async (_name, status) => {
    await act(async () => {
      root.render(
        <UpdateModal
          // The union is narrowed by the component at runtime; these fixtures
          // intentionally cover every non-check-error state.
          status={status}
          onDownload={vi.fn()}
          onRetryCheck={vi.fn()}
          onSkip={vi.fn()}
          onDismiss={vi.fn()}
        />,
      );
    });

    expect(container.textContent).not.toContain('Download latest version');
  });
});
