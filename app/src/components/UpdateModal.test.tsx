import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { UpdateStatus } from '../lib/updater';

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
  const callbacks = {
    onDownload: vi.fn(),
    onRestart: vi.fn(),
    onRetryCheck: vi.fn(),
    onDismiss: vi.fn(),
  };

  async function render(status: UpdateStatus) {
    await act(async () => {
      root.render(<UpdateModal status={status} {...callbacks} />);
    });
  }

  function actionLabels() {
    return Array.from(container.querySelectorAll<HTMLButtonElement>('button:not([aria-label])'))
      .map((button) => button.textContent);
  }

  beforeEach(() => {
    vi.clearAllMocks();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it('offers only Download Update and Later for an optional update', async () => {
    await render({
      phase: 'available',
      version: '0.43.1',
      notes: '## New Features\n\n- A useful change.',
      isForced: false,
    });

    expect(actionLabels()).toEqual(['Download Update', 'Later']);
    expect(container.textContent).not.toContain('Skip');

    const [download, later] = Array.from(
      container.querySelectorAll<HTMLButtonElement>('button:not([aria-label])'),
    );
    await act(async () => download?.click());
    await act(async () => later?.click());

    expect(callbacks.onDownload).toHaveBeenCalledOnce();
    expect(callbacks.onDismiss).toHaveBeenCalledOnce();
  });

  it('sanitizes release notes and keeps keyboard focus inside the dialog', async () => {
    vi.useFakeTimers();
    await render({
      phase: 'available',
      version: '0.43.1',
      notes: '[Unsafe link](javascript:alert(1))\n\n<script>alert(2)</script>',
      isForced: false,
    });
    await act(async () => vi.advanceTimersByTime(60));

    const dialog = container.querySelector<HTMLDivElement>('[role="dialog"]');
    const unsafeLink = Array.from(container.querySelectorAll('a')).find(
      (link) => link.textContent === 'Unsafe link',
    );
    expect(dialog?.getAttribute('aria-modal')).toBe('true');
    expect(dialog?.className).toContain('h-[min(620px,calc(100vh-2.5rem))]');
    expect(unsafeLink?.getAttribute('href') ?? '').not.toMatch(/^javascript:/);
    expect(container.querySelector('script')).toBeNull();
    expect(document.activeElement?.textContent).toBe('Download Update');

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
    });
    expect(document.activeElement?.textContent).toBe('Later');

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(callbacks.onDismiss).toHaveBeenCalledOnce();
  });

  it('shows compact determinate and indeterminate download progress without actions', async () => {
    await render({ phase: 'downloading', version: '0.43.1', progress: 42 });

    const dialog = container.querySelector<HTMLDivElement>('[role="dialog"]');
    const progress = container.querySelector<HTMLElement>('[role="progressbar"]');
    expect(container.textContent).toContain('Downloading Update');
    expect(container.textContent).toContain('42%');
    expect(dialog?.className).toContain('max-h-[calc(100vh-2.5rem)]');
    expect(progress?.getAttribute('aria-valuemin')).toBe('0');
    expect(progress?.getAttribute('aria-valuemax')).toBe('100');
    expect(progress?.getAttribute('aria-valuenow')).toBe('42');
    expect(container.querySelector('button')).toBeNull();

    await render({ phase: 'downloading', version: '0.43.1', progress: null });
    const indeterminateProgress = container.querySelector<HTMLElement>('[role="progressbar"]');
    expect(indeterminateProgress?.hasAttribute('aria-valuenow')).toBe(false);
    expect(container.textContent).not.toContain('null%');
    expect(container.querySelector('button')).toBeNull();
  });

  it('keeps focus trapped when a busy state has no buttons', async () => {
    vi.useFakeTimers();
    const outsideButton = document.createElement('button');
    document.body.appendChild(outsideButton);
    await render({ phase: 'preparing', version: '0.43.1' });
    await act(async () => vi.advanceTimersByTime(60));

    const dialog = container.querySelector<HTMLDivElement>('[role="dialog"]');
    expect(container.textContent).toContain('Preparing Update');
    expect(document.activeElement).toBe(dialog);

    outsideButton.focus();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
    });
    expect(document.activeElement).toBe(dialog);

    await act(async () => {
      container.firstElementChild?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    expect(callbacks.onDismiss).not.toHaveBeenCalled();
    outsideButton.remove();
  });

  it('moves focus from download progress to ready actions and can reopen ready state', async () => {
    vi.useFakeTimers();
    await render({ phase: 'downloading', version: '0.43.1', progress: 100 });
    await act(async () => vi.advanceTimersByTime(60));

    await render({ phase: 'ready', version: '0.43.1', isForced: false });
    await act(async () => vi.advanceTimersByTime(60));

    expect(container.textContent).toContain('Ready to Restart');
    expect(container.textContent).toContain('Restart Murmur to install the downloaded update.');
    expect(actionLabels()).toEqual(['Restart Now', 'Later']);
    expect(document.activeElement?.textContent).toBe('Restart Now');

    await act(async () => {
      Array.from(container.querySelectorAll('button')).find(
        (button) => button.textContent === 'Restart Now',
      )?.click();
    });
    expect(callbacks.onRestart).toHaveBeenCalledOnce();

    await render({ phase: 'idle' });
    await render({ phase: 'ready', version: '0.43.1', isForced: false });
    await act(async () => vi.advanceTimersByTime(60));
    expect(actionLabels()).toEqual(['Restart Now', 'Later']);
    expect(document.activeElement?.textContent).toBe('Restart Now');
  });

  it('uses Quit for required updates and does not allow dismissal', async () => {
    await render({ phase: 'ready', version: '0.43.1', isForced: true });

    expect(actionLabels()).toEqual(['Restart Now', 'Quit']);
    expect(container.querySelector('[aria-label="Close update dialog"]')).toBeNull();

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      container.firstElementChild?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      Array.from(container.querySelectorAll('button')).find(
        (button) => button.textContent === 'Quit',
      )?.click();
    });
    expect(callbacks.onDismiss).not.toHaveBeenCalled();
    expect(mocks.exit).toHaveBeenCalledWith(0);
  });

  it('shows restart progress without actions or dismissal', async () => {
    await render({ phase: 'restarting', version: '0.43.1' });

    expect(container.textContent).toContain('Restarting Murmur');
    expect(container.textContent).toContain('Installing the update and restarting...');
    expect(container.querySelector('button')).toBeNull();

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(callbacks.onDismiss).not.toHaveBeenCalled();
  });

  it.each([
    ['check', 'onRetryCheck'],
    ['install', 'onDownload'],
    ['restart', 'onRestart'],
  ] as const)('routes %s failures to %s', async (stage, callbackName) => {
    await render({
      phase: 'error',
      stage,
      message: `${stage} failed`,
      isForced: false,
    });

    expect(actionLabels()).toEqual(['Retry', 'Later']);
    await act(async () => {
      Array.from(container.querySelectorAll('button')).find(
        (button) => button.textContent === 'Retry',
      )?.click();
    });

    expect(callbacks[callbackName]).toHaveBeenCalledOnce();
  });

  it('keeps manual release-page recovery for check failures', async () => {
    await render({
      phase: 'error',
      stage: 'check',
      message: 'Could not check for updates.',
      isForced: false,
    });

    const manualDownload = Array.from(container.querySelectorAll('a')).find(
      (link) => link.textContent === 'download the latest version manually',
    );
    expect(manualDownload?.getAttribute('href')).toBe(LATEST_RELEASES_URL);
    expect(actionLabels()).toEqual(['Retry', 'Later']);

    await act(async () => manualDownload?.click());
    expect(mocks.openUrl).toHaveBeenCalledWith(LATEST_RELEASES_URL);
  });

  it('offers Quit and Later instead of retrying from a read-only install location', async () => {
    const message =
      'macOS opened Murmur from a read-only security location. Quit Murmur, then move it to Applications.';
    await render({
      phase: 'error',
      stage: 'install',
      message,
      isForced: false,
      recovery: 'reinstall',
    });

    expect(container.textContent).toContain('Reinstall Murmur to Update');
    expect(container.textContent).toContain(message);
    expect(actionLabels()).toEqual(['Quit', 'Later']);

    const [quit, later] = Array.from(
      container.querySelectorAll<HTMLButtonElement>('button:not([aria-label])'),
    );
    await act(async () => quit?.click());
    await act(async () => later?.click());

    expect(mocks.exit).toHaveBeenCalledWith(0);
    expect(callbacks.onDismiss).toHaveBeenCalledOnce();
    expect(callbacks.onDownload).not.toHaveBeenCalled();
    expect(callbacks.onRestart).not.toHaveBeenCalled();
    expect(callbacks.onRetryCheck).not.toHaveBeenCalled();
  });

  it('only allows quitting for a required reinstall recovery', async () => {
    await render({
      phase: 'error',
      stage: 'install',
      message: 'Move Murmur to Applications.',
      isForced: true,
      recovery: 'reinstall',
    });

    expect(actionLabels()).toEqual(['Quit']);
    expect(container.querySelector('[aria-label="Close update dialog"]')).toBeNull();

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      container.firstElementChild?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    expect(callbacks.onDismiss).not.toHaveBeenCalled();
  });

  it.each([
    ['idle', { phase: 'idle' }],
    ['checking', { phase: 'checking' }],
    ['up-to-date', { phase: 'up-to-date' }],
  ] satisfies ReadonlyArray<readonly [string, UpdateStatus]>)('stays hidden in %s state', async (_name, status) => {
    await render(status);
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });
});
