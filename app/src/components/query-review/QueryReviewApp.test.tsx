import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  signIn: vi.fn(async () => undefined),
  driver: {
    state: 'failed' as 'failed' | 'ready' | 'listening',
    errorCode: 'provider_not_authenticated' as string | null,
    answer: 'partial stdout must not mask failure',
    partial: '',
    errorDetail: 'Error: Not logged in' as string | null,
    usage: null as null | {
      inputTokens: number;
      outputTokens: number;
      reasoningOutputTokens: number;
      cachedInputTokens: number;
      cacheCreationInputTokens: number;
      costUsd: number | null;
    },
    signInFix: 'Run claude /login in Terminal.',
    signInStatus: null,
    signInBusy: false,
    contextSummary: null as string | null,
    cancel: vi.fn(),
    copy: vi.fn(),
    signIn: vi.fn(async () => undefined),
  },
}));

vi.mock('../../lib/hooks/useQueryReviewDriver', () => ({
  useQueryReviewDriver: () => mocks.driver,
}));

import {
  QueryReviewApp,
  formatQueryUsage,
  queryErrorMessage,
} from './QueryReviewApp';

describe('QueryReviewApp', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    mocks.driver.state = 'failed';
    mocks.driver.errorCode = 'provider_not_authenticated';
    mocks.driver.answer = 'partial stdout must not mask failure';
    mocks.driver.partial = '';
    mocks.driver.errorDetail = 'Error: Not logged in';
    mocks.driver.usage = null;
    mocks.driver.contextSummary = null;
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

  it('maps declared-environment failures to actionable Settings recovery', () => {
    expect(queryErrorMessage('invalid_environment')).toBe(
      'The saved Voice Query environment is invalid. Clear and re-enter it in Settings.',
    );
    expect(queryErrorMessage('environment_unavailable')).toBe(
      'Murmur could not read the protected Voice Query environment. Open Settings and clear or re-save it.',
    );
  });

  it('maps a terminal capture failure to actionable microphone guidance', () => {
    expect(queryErrorMessage('audio_capture_failed')).toBe(
      'Microphone capture failed while stopping. Check the selected input and try again.',
    );
  });

  it('diagnoses a missing Codex platform binary from bounded provider detail', () => {
    expect(queryErrorMessage(
      'exit_nonzero',
      'Error: spawn /opt/homebrew/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/codex/codex ENOENT',
    )).toBe('The Codex CLI installation is incomplete. Reinstall or update Codex, then try again.');
    expect(queryErrorMessage(
      'exit_nonzero',
      'The Codex CLI installation is incomplete. Reinstall or update Codex, then try again.',
    )).toBe('The Codex CLI installation is incomplete. Reinstall or update Codex, then try again.');
  });

  it('formats provider-reported tokens and optional cost', () => {
    expect(formatQueryUsage({
      inputTokens: 1234,
      outputTokens: 56,
      reasoningOutputTokens: 0,
      cachedInputTokens: 100,
      cacheCreationInputTokens: 2,
      costUsd: 0.012,
    })).toBe('1,234 in · 56 out · $0.012');
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

  it('replaces the exact nested Codex ENOENT stack without rendering raw provider detail', async () => {
    mocks.driver.errorCode = 'exit_nonzero';
    mocks.driver.errorDetail = 'Error: spawn /opt/homebrew/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/codex/codex ENOENT';
    await act(async () => root.render(<QueryReviewApp />));

    expect(container.textContent).toContain('The Codex CLI installation is incomplete');
    expect(container.textContent).not.toContain('Provider detail');
    expect(container.textContent).not.toContain('/opt/homebrew');
    expect(container.textContent).not.toContain('ENOENT');
  });

  it('shows pass-scoped usage in the Ready footer', async () => {
    mocks.driver.state = 'ready';
    mocks.driver.errorCode = null;
    mocks.driver.answer = 'answer';
    mocks.driver.errorDetail = null;
    mocks.driver.usage = {
      inputTokens: 21,
      outputTokens: 13,
      reasoningOutputTokens: 5,
      cachedInputTokens: 8,
      cacheCreationInputTokens: 2,
      costUsd: 0.0004,
    };
    await act(async () => root.render(<QueryReviewApp />));

    expect(container.textContent).toContain('21 in · 13 out · $0.0004');
    expect(container.textContent).toContain('Never auto-pasted');
  });

  it('keeps disabled and unavailable automatic copy states truthful and actionable', async () => {
    mocks.driver.state = 'ready';
    mocks.driver.errorCode = 'auto_copy_disabled';
    mocks.driver.answer = 'answer';
    mocks.driver.errorDetail = null;
    await act(async () => root.render(<QueryReviewApp />));

    expect(container.textContent).toContain('Answer ready');
    expect(container.textContent).toContain('Automatic copy off');
    expect(container.textContent).toContain('press Copy for the answer');
    expect(container.textContent).not.toContain('Answer copied to clipboard');

    mocks.driver.errorCode = 'auto_copy_unavailable';
    await act(async () => root.render(<QueryReviewApp />));
    expect(container.textContent).toContain('Automatic copy unavailable');
    expect(container.textContent).not.toContain('Answer copied to clipboard');
  });

  it('shows listening words as plain text before the CLI runs', async () => {
    mocks.driver.state = 'listening';
    mocks.driver.errorCode = null;
    mocks.driver.answer = '';
    mocks.driver.partial = 'what is the weather';
    mocks.driver.errorDetail = null;
    await act(async () => root.render(<QueryReviewApp />));

    const heard = container.querySelector('[aria-label="Heard so far"]');
    expect(heard?.textContent).toBe('what is the weather');
    expect(container.querySelector('[aria-label="Query answer"]')).toBeNull();
  });
});
