import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { useQueryHistory } from '../../lib/hooks/useQueryHistory';
import { QueryHistoryPanel } from './QueryHistoryPanel';

function history(overrides: Partial<ReturnType<typeof useQueryHistory>> = {}): ReturnType<typeof useQueryHistory> {
  return {
    entries: [{
      schemaVersion: 1,
      id: '0123456789abcdef0123456789abcdef',
      timestampMs: Date.UTC(2026, 7, 12, 12),
      provider: 'claude',
      question: 'What is private?',
      answer: 'Context and stderr never enter this store.',
      tokens: {
        inputTokens: 12,
        outputTokens: 9,
        reasoningOutputTokens: 2,
        cachedInputTokens: 3,
        cacheCreationInputTokens: 1,
      },
      durationMs: 1_250,
      errorCode: null,
    }],
    provider: 'all',
    search: '',
    total: 1,
    hasMore: false,
    loading: false,
    clearing: false,
    error: null,
    setProvider: vi.fn(),
    setSearch: vi.fn(),
    refresh: vi.fn(async () => {}),
    loadMore: vi.fn(async () => {}),
    clear: vi.fn(async () => true),
    ...overrides,
  };
}

describe('QueryHistoryPanel', () => {
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

  it('shows local question/answer records and explains the context boundary', async () => {
    await act(async () => root.render(<QueryHistoryPanel history={history()} retentionEnabled />));
    expect(container.textContent).toContain('What is private?');
    expect(container.textContent).toContain('Context and stderr never enter this store.');
    expect(container.textContent).toContain('12 in · 9 out · 3 cached · 1 cache write · 2 reasoning');
    expect(container.textContent).toContain('Context is never stored as a separate field; provider stderr, commands, paths, and environment values are never stored here.');
    expect(container.textContent).not.toContain('$');
  });

  it('presents successful clipboard delivery states as neutral notices', async () => {
    const base = history().entries[0];
    await act(async () => root.render(
      <QueryHistoryPanel
        history={history({
          entries: [
            { ...base, id: '11111111111111111111111111111111', errorCode: 'auto_copy_disabled' },
            { ...base, id: '22222222222222222222222222222222', errorCode: 'clipboard_superseded' },
          ],
          total: 2,
        })}
        retentionEnabled
      />,
    ));

    expect(container.textContent).toContain('Manual copy');
    expect(container.textContent).toContain('Clipboard unchanged');
    expect(container.textContent).not.toContain('auto_copy_disabled');
    expect(container.textContent).not.toContain('clipboard_superseded');
    const notices = Array.from(container.querySelectorAll('article span.ml-auto'));
    expect(notices).toHaveLength(2);
    expect(notices.every((notice) => !notice.classList.contains('text-error'))).toBe(true);
  });

  it('filters by provider and requires an explicit scope-naming confirmation before purging', async () => {
    const state = history();
    const confirm = vi.spyOn(window, 'confirm');
    await act(async () => root.render(<QueryHistoryPanel history={state} retentionEnabled />));

    const select = container.querySelector('select') as HTMLSelectElement;
    await act(async () => {
      select.value = 'codex';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    const purge = Array.from(container.querySelectorAll('button'))
      .find((button) => button.textContent === 'Delete all query history')!;
    await act(async () => {
      purge.click();
      await Promise.resolve();
    });

    expect(state.setProvider).toHaveBeenCalledWith('codex');
    expect(state.clear).not.toHaveBeenCalled();
    const dialog = container.querySelector('[role="dialog"]')!;
    expect(dialog.textContent).toContain('including entries hidden by search or provider filters');
    expect(dialog.textContent).toContain('Dictation and meeting history are not affected');
    expect(document.activeElement?.textContent).toBe('Cancel');
    const confirmKey = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true });
    await act(async () => dialog.querySelectorAll('button')[1].dispatchEvent(confirmKey));
    expect(confirmKey.defaultPrevented).toBe(true);
    expect(state.clear).not.toHaveBeenCalled();
    await act(async () => {
      (dialog.querySelectorAll('button')[1] as HTMLButtonElement).click();
      await Promise.resolve();
    });
    expect(state.clear).toHaveBeenCalledOnce();
    expect(confirm).not.toHaveBeenCalled();
    expect(container.textContent).toContain('Voice Query history deleted from this Mac.');
  });

  it('cancels with no clear call, restores focus, and supports Escape dismissal', async () => {
    const state = history();
    await act(async () => root.render(<QueryHistoryPanel history={state} retentionEnabled />));
    const opener = container.querySelector('[aria-label="Delete all Voice Query history"]') as HTMLButtonElement;
    (container.querySelector('input[type="search"]') as HTMLInputElement).focus();
    await act(async () => opener.click());
    await act(async () => (container.querySelector('[role="dialog"] button') as HTMLButtonElement).click());
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(opener);
    await act(async () => opener.click());
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(state.clear).not.toHaveBeenCalled();
  });

  it('clears exactly once while pending, blocks busy dismissal, and reports failure without success', async () => {
    let complete!: (value: boolean) => void;
    const state = history({ clear: vi.fn(() => new Promise<boolean>(resolve => { complete = resolve; })) });
    await act(async () => root.render(<QueryHistoryPanel history={state} retentionEnabled />));
    await act(async () => (container.querySelector('[aria-label="Delete all Voice Query history"]') as HTMLButtonElement).click());
    const confirm = container.querySelector('[role="dialog"] button:last-child') as HTMLButtonElement;
    await act(async () => { confirm.click(); confirm.click(); });
    expect(state.clear).toHaveBeenCalledOnce();
    expect(confirm.disabled).toBe(true);
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(container.querySelector('[role="dialog"]')).not.toBeNull();
    await act(async () => complete(false));
    expect(container.querySelector('[role="dialog"] [role="alert"]')?.textContent).toContain('Could not delete');
    expect(container.textContent).not.toContain('history deleted from this Mac');
    expect(confirm.disabled).toBe(false);
    expect(container.textContent).toContain('What is private?');
  });

  it('does not open or dispatch clear while the history store is already busy', async () => {
    const state = history({ clearing: true });
    await act(async () => root.render(<QueryHistoryPanel history={state} retentionEnabled />));
    const opener = container.querySelector('[aria-label="Delete all Voice Query history"]') as HTMLButtonElement;
    expect(opener.disabled).toBe(true);
    await act(async () => opener.click());
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(state.clear).not.toHaveBeenCalled();
  });

  it('searches questions and answers and offers a clear affordance', async () => {
    const state = history({ search: 'private answer' });
    await act(async () => root.render(<QueryHistoryPanel history={state} retentionEnabled />));

    const search = container.querySelector(
      'input[aria-label="Search Voice Query history"]',
    ) as HTMLInputElement;
    expect(search.value).toBe('private answer');
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(search, 'different');
      search.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(state.setSearch).toHaveBeenCalledWith('different');

    const clear = container.querySelector(
      'button[aria-label="Clear Voice Query history search"]',
    ) as HTMLButtonElement;
    await act(async () => clear.click());
    expect(state.setSearch).toHaveBeenCalledWith('');
  });

  it('distinguishes no search matches from an empty saved-query store', async () => {
    await act(async () => root.render(
      <QueryHistoryPanel
        history={history({ entries: [], search: 'unmatched', total: 0 })}
        retentionEnabled
      />,
    ));

    expect(container.textContent).toContain('No matches for your search');
    expect(container.textContent).toContain('Try a different word or clear the search.');
    expect(container.textContent).not.toContain('No saved voice queries');
  });

  it('explains the fail-closed default when retention is off', async () => {
    await act(async () => root.render(
      <QueryHistoryPanel history={history({ entries: [], total: 0 })} retentionEnabled={false} />,
    ));
    expect(container.textContent).toContain('Saving is off. Existing entries remain available until you delete them.');
    expect(container.textContent).toContain('Turn on “Keep Voice Query history on this Mac”');
  });

  it('keeps purge available when the visible total is zero', async () => {
    const state = history({ entries: [], total: 0 });
    await act(async () => root.render(<QueryHistoryPanel history={state} retentionEnabled />));
    const purge = Array.from(container.querySelectorAll('button'))
      .find((button) => button.textContent === 'Delete all query history') as HTMLButtonElement;
    expect(purge.disabled).toBe(false);
    await act(async () => {
      purge.click();
      await Promise.resolve();
    });
    expect(state.clear).not.toHaveBeenCalled();
    await act(async () => {
      (container.querySelector('[role="dialog"] button:last-child') as HTMLButtonElement).click();
      await Promise.resolve();
    });
    expect(state.clear).toHaveBeenCalledOnce();
    expect(container.textContent).toContain('Voice Query history deleted from this Mac.');
  });

  it('explains that recognized context-bearing queries are retained', async () => {
    const state = history({ entries: [], total: 0 });
    await act(async () => root.render(<QueryHistoryPanel history={state} retentionEnabled />));
    expect(container.textContent).toContain('including queries that shared app context');
    expect(container.textContent).toContain('Saved answers can quote that context.');
  });

  it('turns a saved provider failure into an explanation and next step', async () => {
    const failed = {
      ...history().entries[0],
      provider: 'codex' as const,
      answer: '',
      errorCode: 'exit_nonzero' as const,
    };
    await act(async () => root.render(
      <QueryHistoryPanel history={history({ entries: [failed] })} retentionEnabled />,
    ));

    expect(container.textContent).toContain('Failed');
    expect(container.textContent).toContain('The configured CLI exited with an error.');
    expect(container.textContent).toContain('Open Settings > AI & Models > Voice Query and run Test');
    expect(container.textContent).not.toContain('exit_nonzero');
    expect(container.textContent).not.toContain('No answer was returned');
  });

  it('distinguishes an empty provider filter from an empty history store', async () => {
    await act(async () => root.render(
      <QueryHistoryPanel history={history({ entries: [], provider: 'codex', total: 0 })} retentionEnabled />,
    ));

    expect(container.textContent).toContain('No saved Codex queries');
    expect(container.textContent).toContain('Choose All providers');
    expect(container.textContent).not.toContain('No saved voice queries');
  });
});
