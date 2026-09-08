import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ReviewDriverResult } from '../../lib/hooks/useTransformReviewDriver';

const hooks = vi.hoisted(() => ({
  driver: { current: null as ReviewDriverResult | null },
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('../../lib/hooks/useTransformReviewDriver', () => ({
  useTransformReviewDriver: () => hooks.driver.current,
}));
vi.mock('../../lib/hooks/useTransformReviewMockDriver', () => ({
  isMockReviewEnabled: () => false,
  useMockReviewDriver: () => hooks.driver.current,
}));

import { TransformReviewApp } from './TransformReviewApp';

function driver(overrides: Partial<ReviewDriverResult> = {}): ReviewDriverResult {
  return {
    state: 'ready',
    errorCode: null,
    content: { correction: 'selection', instruction: 'change Friday to Monday', original: '', proposed: '' },
    thinkingElapsedMs: 0,
    cancel: vi.fn(),
    retry: vi.fn(),
    approve: vi.fn(),
    undo: vi.fn(),
    finish: vi.fn(),
    ...overrides,
  };
}

describe('TransformReviewApp correction teaching gate', () => {
  let container: HTMLDivElement;
  let root: Root;
  const originalMatchMedia = window.matchMedia;
  beforeEach(() => {
    // Reduced motion: the shell's entrance/dismiss timers are irrelevant here.
    window.matchMedia = vi.fn(() => ({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })) as unknown as typeof window.matchMedia;
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    window.matchMedia = originalMatchMedia;
    hooks.driver.current = null;
  });

  it('hides "Remember this correction" until the pulled content has arrived', async () => {
    // The popover reaches `ready` on the state event; original/proposed text
    // is fetched separately. Offering teaching in that window would propose a
    // learned correction with an empty replacement.
    hooks.driver.current = driver();
    await act(async () => root.render(<TransformReviewApp />));
    expect(container.textContent).not.toContain('Remember this correction');
  });

  it('offers teaching once the correction content is present', async () => {
    hooks.driver.current = driver({
      content: {
        correction: 'selection',
        instruction: 'change Friday to Monday',
        original: 'Ship the release on Friday',
        proposed: 'Ship the release on Monday',
      },
    });
    await act(async () => root.render(<TransformReviewApp />));
    expect(container.textContent).toContain('Remember this correction');
  });
});
