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
    total: 1,
    hasMore: false,
    loading: false,
    clearing: false,
    error: null,
    setProvider: vi.fn(),
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

  it('filters by provider and purges immediately without a modal confirmation', async () => {
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
    expect(state.clear).toHaveBeenCalledOnce();
    expect(confirm).not.toHaveBeenCalled();
    expect(container.textContent).toContain('Voice Query history deleted from this Mac.');
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
    expect(state.clear).toHaveBeenCalledOnce();
    expect(container.textContent).toContain('Voice Query history deleted from this Mac.');
  });

  it('explains that recognized context-bearing queries are retained', async () => {
    const state = history({ entries: [], total: 0 });
    await act(async () => root.render(<QueryHistoryPanel history={state} retentionEnabled />));
    expect(container.textContent).toContain('including queries that shared app context');
    expect(container.textContent).toContain('Saved answers can quote that context.');
  });
});
