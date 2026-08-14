import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PerformanceStoreHealthV1 } from '../../lib/performance';
import { PerformanceStoreHealthBanner } from './PerformanceStoreHealthBanner';

const AVAILABLE: PerformanceStoreHealthV1 = {
  schemaVersion: 1,
  status: 'available',
  skippedRunCount: 0,
  recommendedAction: 'none',
};

describe('PerformanceStoreHealthBanner', () => {
  let container: HTMLDivElement;
  let root: Root;
  let onRefresh: ReturnType<typeof vi.fn<() => void>>;
  let onRecover: ReturnType<typeof vi.fn<() => void>>;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    onRefresh = vi.fn<() => void>();
    onRecover = vi.fn<() => void>();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  async function render(health: PerformanceStoreHealthV1 | null, overrides: {
    loading?: boolean;
    error?: string | null;
    recovering?: boolean;
    recoveryError?: string | null;
  } = {}) {
    await act(async () => {
      root.render(
        <PerformanceStoreHealthBanner
          health={health}
          loading={overrides.loading ?? false}
          error={overrides.error ?? null}
          recovering={overrides.recovering ?? false}
          recoveryError={overrides.recoveryError ?? null}
          onRefresh={onRefresh}
          onRecover={onRecover}
        />,
      );
    });
  }

  it('distinguishes a skipped run from an unavailable store', async () => {
    await render({
      ...AVAILABLE,
      skippedRunCount: 1,
      lastFailure: {
        operation: 'begin',
        errorClass: 'busyLocked',
        attemptCount: 3,
        retryExhausted: true,
        atMs: 1_786_720_000_000,
        recordingId: 35,
      },
    });
    expect(container.textContent).toContain('1 diagnostics run was skipped');
    expect(container.textContent).toContain('Dictation continued normally');
    expect(container.textContent).toContain('store is available now');
    expect(container.textContent).toContain('after 3 attempts');
    expect(container.textContent).not.toContain('unavailable');

    await render({
      ...AVAILABLE,
      status: 'unavailable',
      recommendedAction: 'freeDisk',
    });
    expect(container.textContent).toContain('Diagnostics store unavailable');
    expect(container.textContent).toContain('Free local disk space, then retry');
    expect(container.textContent).toContain('new diagnostics are not being saved');
  });

  it('requires exact confirmation before quarantining and reinitializing', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    await render({
      ...AVAILABLE,
      status: 'unavailable',
      recommendedAction: 'reinitializeStore',
    });

    const button = Array.from(container.querySelectorAll('button'))
      .find(candidate => candidate.textContent === 'Reinitialize Store…')!;
    await act(async () => button.click());
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining(
      'This removes Performance runs and resource samples only',
    ));
    expect(onRecover).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    await act(async () => button.click());
    expect(onRecover).toHaveBeenCalledOnce();
  });

  it('does not report healthy when a later diagnostics write exhausted retries', async () => {
    await render({
      ...AVAILABLE,
      lastFailure: {
        operation: 'complete',
        errorClass: 'busyLocked',
        attemptCount: 3,
        retryExhausted: true,
        atMs: 1_786_720_000_000,
      },
    });
    expect(container.textContent).toContain('Recent diagnostics data was not saved');
    expect(container.textContent).toContain('latest run completion did not finish');
    expect(container.textContent).toContain('after 3 attempts');
    expect(container.textContent).not.toContain('New content-free performance runs');
  });

  it('keeps cumulative skipped runs separate from a newer unsaved write', async () => {
    await render({
      ...AVAILABLE,
      skippedRunCount: 2,
      lastFailure: {
        operation: 'update',
        errorClass: 'busyLocked',
        attemptCount: 3,
        retryExhausted: true,
        atMs: 1_786_720_000_000,
      },
    });

    expect(container.textContent).toContain('2 diagnostics runs were skipped');
    expect(container.textContent).toContain('Recent diagnostics data was not saved');
    expect(container.textContent).toContain('latest run update did not finish');
    expect(container.textContent).not.toContain('run update was skipped');
  });

  it('shows bounded recovery evidence without exposing a path or raw error', async () => {
    await render({
      ...AVAILABLE,
      lastRecovery: {
        action: 'quarantinedAndReinitialized',
        atMs: 1_786_720_000_000,
      },
    });
    expect(container.textContent).toContain('Diagnostics store recovered');
    expect(container.textContent).toContain('quarantined an unreadable diagnostics store');
    expect(container.textContent).not.toContain('/');
    expect(container.textContent).not.toContain('SQLITE');
  });

  it('offers refresh when health itself cannot be verified', async () => {
    await render(null, {
      error: 'Murmur could not verify the local diagnostics store.',
    });
    expect(container.textContent).toContain('Diagnostics health could not be verified');
    const refresh = Array.from(container.querySelectorAll('button'))
      .find(candidate => candidate.textContent === 'Refresh')!;
    await act(async () => refresh.click());
    expect(onRefresh).toHaveBeenCalledOnce();
  });

  it('does not present a stale healthy snapshot after verification fails', async () => {
    await render(AVAILABLE, {
      error: 'Murmur could not verify the local diagnostics store.',
    });
    expect(container.textContent).toContain('Diagnostics health could not be verified');
    expect(container.textContent).not.toContain('Diagnostics store available');
  });
});
