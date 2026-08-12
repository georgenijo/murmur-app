import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  signIn: vi.fn(async () => undefined),
  driver: {
    state: 'failed' as const,
    errorCode: 'provider_not_authenticated',
    answer: 'partial stdout must not mask failure',
    errorDetail: 'Error: Not logged in',
    signInFix: 'Run claude /login in Terminal.',
    signInStatus: null,
    signInBusy: false,
    cancel: vi.fn(),
    copy: vi.fn(),
    signIn: vi.fn(async () => undefined),
  },
}));

vi.mock('../../lib/hooks/useQueryReviewDriver', () => ({
  useQueryReviewDriver: () => mocks.driver,
}));

import { QueryReviewApp, queryErrorMessage } from './QueryReviewApp';

describe('QueryReviewApp', () => {
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

  it('maps typed provider failures without exposing their detail in the event', () => {
    expect(queryErrorMessage('provider_error')).toBe('The configured provider reported an error.');
  });

  it('shows terminal failure ahead of partial stdout and keeps stderr distinct', async () => {
    await act(async () => root.render(<QueryReviewApp />));

    expect(container.textContent).toContain('The configured provider is not signed in.');
    expect(container.textContent).not.toContain('partial stdout must not mask failure');
    expect(container.textContent).toContain('Provider detail');
    expect(container.textContent).toContain('Error: Not logged in');
    expect(container.textContent).toContain('Run claude /login in Terminal.');
    expect(Array.from(container.querySelectorAll('button')).some(
      (button) => button.textContent === 'Sign in…',
    )).toBe(true);
  });
});
