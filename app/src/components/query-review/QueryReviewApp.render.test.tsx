import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  driver: vi.fn(),
}));

vi.mock('../../lib/hooks/useQueryReviewDriver', () => ({
  useQueryReviewDriver: mocks.driver,
}));

import { QueryReviewApp } from './QueryReviewApp';

interface DriverState {
  state: string;
  errorCode: string | null;
  answer: string;
  errorDetail: string | null;
  signIn: { provider: string; hint: string } | null;
}

function driverWith(overrides: Partial<DriverState>) {
  return {
    state: 'failed',
    errorCode: null,
    answer: '',
    errorDetail: null,
    signIn: null,
    cancel: vi.fn(),
    copy: vi.fn(),
    startSignIn: vi.fn(),
    ...overrides,
  };
}

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
    mocks.driver.mockReset();
  });

  async function render(overrides: Partial<DriverState>) {
    const driver = driverWith(overrides);
    mocks.driver.mockReturnValue(driver);
    await act(async () => { root.render(<QueryReviewApp />); });
    return driver;
  }

  it('shows the failure, not the stdout the CLI printed before giving up', async () => {
    // The Claude "Not logged in" incident: the provider printed its refusal on
    // stdout and exited non-zero. `answer || errorMessage` rendered that stdout
    // as though it were the answer, hiding the real error entirely.
    await render({
      state: 'failed',
      errorCode: 'provider_not_authenticated',
      answer: 'Not logged in',
      errorDetail: 'error: credentials not found',
      signIn: { provider: 'Claude Code', hint: 'claude auth login' },
    });
    const text = container.textContent ?? '';
    expect(text).toContain('The CLI is not signed in, so it refused the question.');
    expect(text).toContain('Run claude auth login in Terminal');
    expect(text).toContain('error: credentials not found');
    // The stdout is still available, but labelled as evidence rather than shown
    // as the answer.
    expect(text).toContain('Partial output');
    expect(container.querySelector('[aria-label="Query answer"] p')?.textContent)
      .toBe('The CLI is not signed in, so it refused the question.');
  });

  it('offers the vendor sign-in only for a signed-out pass with a known provider', async () => {
    const authenticated = await render({
      state: 'failed',
      errorCode: 'provider_not_authenticated',
      signIn: { provider: 'Claude Code', hint: 'claude auth login' },
    });
    const signInButton = [...container.querySelectorAll('button')]
      .find((button) => button.textContent === 'Sign in…');
    expect(signInButton).toBeDefined();
    await act(async () => { signInButton?.click(); });
    expect(authenticated.startSignIn).toHaveBeenCalledTimes(1);

    // A custom command has no vendor login, so no button is offered.
    await act(async () => root.unmount());
    root = createRoot(container);
    await render({ state: 'failed', errorCode: 'provider_not_authenticated', signIn: null });
    expect([...container.querySelectorAll('button')].some((button) => button.textContent === 'Sign in…'))
      .toBe(false);

    // Nor for an unrelated failure with a known provider.
    await act(async () => root.unmount());
    root = createRoot(container);
    await render({
      state: 'failed',
      errorCode: 'timed_out',
      signIn: { provider: 'Claude Code', hint: 'claude auth login' },
    });
    expect([...container.querySelectorAll('button')].some((button) => button.textContent === 'Sign in…'))
      .toBe(false);
  });

  it('still renders a successful answer as markdown', async () => {
    await render({ state: 'ready', errorCode: null, answer: '# Heading\n\nThe answer.' });
    expect(container.querySelector('[aria-label="Query answer"] h1')?.textContent).toBe('Heading');
    expect(container.textContent).toContain('The answer.');
  });
});
