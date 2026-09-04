import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HistoryPanel } from './HistoryPanel';
import { type HistoryEntry } from '../../lib/history';

const invoke = vi.fn().mockResolvedValue(0);
const save = vi.fn().mockResolvedValue('/Users/me/Documents/murmur-history.md');

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: (...args: unknown[]) => save(...args) }));

// The Pointer Capture API stub used by HoldToDeleteButton's synthetic
// PointerEvents lives in vitest.setup.ts (shared across test files).

function entry(overrides: Partial<HistoryEntry> & { id: string }): HistoryEntry {
  return {
    text: 'hello world',
    timestamp: Date.UTC(2026, 6, 18, 12),
    duration: 3,
    source: 'recording',
    ...overrides,
  };
}

const ENTRIES: HistoryEntry[] = [
  entry({ id: 'mic', text: 'ship the Tauri release notes' }),
  entry({ id: 'file', text: 'imported meeting audio', source: 'file', sourceName: 'standup.wav' }),
  entry({ id: 'note', text: 'remember the invariant', timestamp: Date.UTC(2026, 6, 18, 13) }),
];

describe('HistoryPanel', () => {
  let container: HTMLDivElement;
  let root: Root;
  let writeText: ReturnType<typeof vi.fn>;

  const buttons = () => Array.from(document.querySelectorAll<HTMLElement>('button, [role="menuitem"]'));
  const byText = (text: string) => buttons().find((b) => b.textContent === text);
  const cardText = () => Array.from(container.querySelectorAll('article')).map((a) => a.textContent ?? '');
  const searchShell = () => container.querySelector('[data-testid="history-search-shell"]') as HTMLDivElement;
  const searchInput = () => container.querySelector('input[type="search"]') as HTMLInputElement;
  const searchClose = () => container.querySelector('[aria-label="Clear transcript search"]') as HTMLButtonElement;
  const moreActions = () => container.querySelector('[aria-label="More history actions"]') as HTMLButtonElement;

  async function render(props: Partial<Parameters<typeof HistoryPanel>[0]> = {}) {
    await act(async () => {
      root.render(
        <HistoryPanel
          entries={ENTRIES}
          onClear={vi.fn()}
          onUpdateEntry={vi.fn()}
          {...props}
        />,
      );
    });
  }

  async function type(value: string) {
    const input = searchInput();
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(input, value);
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
  }

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    invoke.mockClear();
    save.mockClear();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('keeps the empty state when there is no history', async () => {
    await render({ entries: [] });
    expect(container.textContent).toContain('No transcription history yet');
    expect(searchInput()).not.toBeNull();
    expect(container.textContent).toContain('0 entries');
  });

  it('orders entries newest first', async () => {
    await render();
    expect(cardText()[0]).toContain('remember the invariant');
  });

  it('renders long history in bounded batches without hiding older entries', async () => {
    const entries = Array.from({ length: 35 }, (_, index) => entry({
      id: `entry-${index}`,
      text: `transcript ${index}`,
      timestamp: Date.UTC(2026, 6, 18, 12, index),
    }));
    await render({ entries });

    expect(cardText()).toHaveLength(30);
    expect(byText('Show 5 older')).toBeTruthy();

    await act(async () => moreActions().click());
    const copyJson = document.querySelector(
      '[aria-label="Copy 35 shown as JSON"]',
    ) as HTMLElement;
    await act(async () => copyJson.click());
    const lastCall = writeText.mock.calls[writeText.mock.calls.length - 1];
    const exported = JSON.parse(lastCall[0] as string) as { count: number };
    expect(exported.count).toBe(35);

    await act(async () => byText('Show 5 older')!.click());
    expect(cardText()).toHaveLength(35);
    expect(byText('Show 5 older')).toBeUndefined();
  });

  it('expands and collapses only overflowing transcripts', async () => {
    vi.stubGlobal('ResizeObserver', class {
      observe() {}
      disconnect() {}
      unobserve() {}
    });
    vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(40);
    vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(function (this: HTMLElement) {
      return this.textContent?.includes('a transcript long enough to overflow') ? 80 : 40;
    });
    await render({
      entries: [
        entry({ id: 'short', text: 'short transcript' }),
        entry({ id: 'long', text: 'a transcript long enough to overflow' }),
      ],
    });

    expect(buttons().filter((button) => button.textContent === 'Show more')).toHaveLength(1);
    await act(async () => byText('Show more')!.click());
    expect(byText('Show less')).toBeTruthy();
    await act(async () => byText('Show less')!.click());
    expect(buttons().filter((button) => button.textContent === 'Show more')).toHaveLength(1);
  });

  it('filters as you type and highlights the match', async () => {
    await render();
    await type('tauri');
    expect(cardText()).toHaveLength(1);
    expect(container.querySelector('mark')?.textContent).toBe('Tauri');
    expect(container.textContent).toContain('1 of 3');
  });

  it('shows an empty-result state that resets the filters', async () => {
    await render();
    await type('nothing matches this');
    expect(container.textContent).toContain('No matching transcripts');
    await act(async () => byText('Reset filters')!.click());
    expect(cardText()).toHaveLength(3);
  });

  it('filters by source', async () => {
    await render();
    const all = byText('All')!;
    const file = byText('File')!;
    expect(all.getAttribute('aria-selected')).toBe('true');
    expect(file.getAttribute('aria-selected')).toBe('false');
    await act(async () => file.click());
    expect(all.getAttribute('aria-selected')).toBe('false');
    expect(file.getAttribute('aria-selected')).toBe('true');
    expect(cardText()).toHaveLength(1);
    expect(cardText()[0]).toContain('standup.wav');
  });

  it('copies only the visible entries as markdown', async () => {
    await render();
    await type('tauri');
    await act(async () => moreActions().click());
    const copyMarkdown = document.querySelector(
      '[aria-label="Copy 1 shown as Markdown"]',
    ) as HTMLElement;
    await act(async () => copyMarkdown.click());
    const calls = writeText.mock.calls;
    const written = calls[calls.length - 1][0] as string;
    expect(written).toContain('# Murmur transcript history');
    expect(written).toContain('ship the Tauri release notes');
    expect(written).not.toContain('imported meeting audio');
    expect(container.textContent).toContain('Copied 1 entry');
  });

  it('reports a per-entry clipboard failure without copying another entry', async () => {
    writeText.mockRejectedValueOnce(new Error('clipboard unavailable'));
    await render();

    const newestCard = Array.from(container.querySelectorAll('article')).find((card) =>
      card.textContent?.includes('remember the invariant'),
    )!;
    await act(async () => (newestCard as HTMLElement).click());

    expect(writeText).toHaveBeenCalledOnce();
    expect(writeText).toHaveBeenCalledWith('remember the invariant');
    expect(container.textContent).toContain('Could not copy to the clipboard.');
    expect(newestCard.getAttribute('data-copied')).toBe('false');
  });

  it('copies the full transcript by clicking the row and reports success', async () => {
    const fullText = 'a complete transcript that can be visually truncated';
    await render({ entries: [entry({ id: 'full', text: fullText })] });
    const card = container.querySelector('[data-testid="transcript-card"]') as HTMLElement;

    await act(async () => card.querySelector('.transcript-text')!.dispatchEvent(new MouseEvent('click', { bubbles: true })));

    expect(writeText).toHaveBeenCalledWith(fullText);
    expect(card.dataset.copied).toBe('true');
    expect(card.textContent).toContain('Copied');
    expect(card.getAttribute('aria-label')).toContain('Press Enter or Space to copy');
  });

  it('reports copy success on a middle row without changing its group position', async () => {
    await render({
      entries: [
        entry({ id: 'oldest', text: 'oldest row', timestamp: Date.UTC(2026, 6, 18, 10) }),
        entry({ id: 'middle', text: 'middle row', timestamp: Date.UTC(2026, 6, 18, 11) }),
        entry({ id: 'newest', text: 'newest row', timestamp: Date.UTC(2026, 6, 18, 12) }),
      ],
    });
    const middleCard = Array.from(container.querySelectorAll('[data-testid="transcript-card"]'))
      .find((card) => card.textContent?.includes('middle row')) as HTMLElement;

    expect(middleCard.dataset.dayEnd).toBe('false');
    await act(async () => middleCard.click());

    expect(middleCard.dataset.copied).toBe('true');
    expect(middleCard.dataset.dayEnd).toBe('false');
    expect(middleCard.querySelector('.transcript-copy-feedback')?.textContent).toBe('Copied');
  });

  it.each(['Enter', ' '])('copies the focused row with %j', async (key) => {
    await render({ entries: [entry({ id: 'keyboard', text: 'keyboard copy' })] });
    const card = container.querySelector('[data-testid="transcript-card"]') as HTMLElement;
    await act(async () => card.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true })));
    expect(writeText).toHaveBeenCalledWith('keyboard copy');
  });

  it('does not copy when nested transcript actions are used', async () => {
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} unobserve() {} });
    vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(20);
    vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockReturnValue(80);
    await render({ entries: [entry({ id: 'nested', text: 'long transcript for nested controls' })] });

    await act(async () => byText('Show more')!.click());
    expect(writeText).not.toHaveBeenCalled();
    const moreTranscriptActions = container.querySelector(
      '[aria-label="More transcript actions"]',
    ) as HTMLButtonElement;
    await act(async () => moreTranscriptActions.click());
    await act(async () => byText('Correct & Teach')!.click());
    expect(writeText).not.toHaveBeenCalled();
  });

  it('saves an export through the native dialog and the validated command', async () => {
    await render();
    await act(async () => moreActions().click());
    const saveJson = document.querySelector(
      '[aria-label="Save 3 shown as JSON"]',
    ) as HTMLElement;
    await act(async () => saveJson.click());
    expect(save).toHaveBeenCalledOnce();
    expect(save.mock.calls[0][0].defaultPath).toMatch(/^murmur-history-.*\.json$/);
    const [command, payload] = invoke.mock.calls[invoke.mock.calls.length - 1];
    expect(command).toBe('save_text_export');
    expect(JSON.parse((payload as { contents: string }).contents).schema).toBe('murmur.history.v2');
    expect(container.textContent).toContain('Saved 3 entries');
  });

  it('says nothing when the save dialog is cancelled', async () => {
    save.mockResolvedValueOnce(null);
    await render();
    await act(async () => moreActions().click());
    const saveMarkdown = document.querySelector(
      '[aria-label="Save 3 shown as Markdown"]',
    ) as HTMLElement;
    await act(async () => saveMarkdown.click());
    expect(invoke.mock.calls.filter(([name]) => name === 'save_text_export')).toHaveLength(0);
    expect(container.textContent).not.toContain('Saved');
  });

  // The hold-to-delete control drives its visible progress fill through
  // Motion's rAF-based animation, which fake timers cannot usefully observe
  // in jsdom. The actual commit path (and this test) is the plain
  // `setTimeout(..., holdDuration)` in HoldToDeleteButton's pointerdown
  // handler, so we assert on that instead of the animated fill.
  it('clears history only after holding the clear control for the full duration', async () => {
    vi.useFakeTimers();
    const onClear = vi.fn();
    await render({ onClear });

    await act(async () => moreActions().click());
    const clear = buttons().find((b) => b.textContent?.includes('Hold to clear history'))!;
    expect(clear).toBeTruthy();

    await act(async () => {
      clear.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }));
    });
    expect(onClear).not.toHaveBeenCalled();

    await act(async () => {
      vi.advanceTimersByTime(1200);
    });
    expect(onClear).toHaveBeenCalledOnce();
    vi.useRealTimers();
  });

  it('does not clear history when the hold is released early', async () => {
    vi.useFakeTimers();
    const onClear = vi.fn();
    await render({ onClear });

    await act(async () => moreActions().click());
    const clear = buttons().find((b) => b.textContent?.includes('Hold to clear history'))!;

    await act(async () => {
      clear.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(300);
      clear.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(onClear).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('cancels an in-progress hold when the window loses focus', async () => {
    vi.useFakeTimers();
    const onClear = vi.fn();
    await render({ onClear });

    await act(async () => moreActions().click());
    const clear = buttons().find((b) => b.textContent?.includes('Hold to clear history'))!;

    await act(async () => {
      clear.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(300);
      window.dispatchEvent(new Event('blur'));
    });
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(onClear).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('cancels an in-progress hold on Escape', async () => {
    vi.useFakeTimers();
    const onClear = vi.fn();
    await render({ onClear });

    await act(async () => moreActions().click());
    const clear = buttons().find((b) => b.textContent?.includes('Hold to clear history'))!;

    await act(async () => {
      clear.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(300);
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(onClear).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('arms a discrete-click confirm state instead of clearing immediately', async () => {
    vi.useFakeTimers();
    const onClear = vi.fn();
    await render({ onClear });

    await act(async () => moreActions().click());
    const clear = buttons().find((b) => b.textContent?.includes('Hold to clear history'))!;

    await act(async () => {
      clear.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(50);
      clear.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1 }));
    });

    expect(onClear).not.toHaveBeenCalled();
    expect(clear.textContent).toContain('Click again to clear history');
    vi.useRealTimers();
  });

  it('clears history on a second discrete click while armed', async () => {
    vi.useFakeTimers();
    const onClear = vi.fn();
    await render({ onClear });

    await act(async () => moreActions().click());
    const clear = buttons().find((b) => b.textContent?.includes('Hold to clear history'))!;

    await act(async () => {
      clear.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(50);
      clear.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1 }));
    });
    expect(onClear).not.toHaveBeenCalled();

    await act(async () => {
      clear.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(50);
      clear.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1 }));
    });
    expect(onClear).toHaveBeenCalledOnce();
    vi.useRealTimers();
  });

  it('auto-disarms the discrete-click confirm state after the timeout', async () => {
    vi.useFakeTimers();
    const onClear = vi.fn();
    await render({ onClear });

    await act(async () => moreActions().click());
    const clear = buttons().find((b) => b.textContent?.includes('Hold to clear history'))!;

    await act(async () => {
      clear.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(50);
      clear.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1 }));
    });
    expect(clear.textContent).toContain('Click again to clear history');

    await act(async () => {
      vi.advanceTimersByTime(4000);
    });
    expect(clear.textContent).toContain('Hold to clear history');

    // A subsequent quick click re-arms rather than committing, since the
    // earlier arm expired.
    await act(async () => {
      clear.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(50);
      clear.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1 }));
    });
    expect(onClear).not.toHaveBeenCalled();
    expect(clear.textContent).toContain('Click again to clear history');
    vi.useRealTimers();
  });

  it('closes the export menu and returns focus to the ··· trigger after a completed hold', async () => {
    vi.useFakeTimers();
    const onClear = vi.fn();
    await render({ onClear });

    await act(async () => moreActions().click());
    const clear = buttons().find((b) => b.textContent?.includes('Hold to clear history'))!;

    await act(async () => {
      clear.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(1200);
    });
    expect(onClear).toHaveBeenCalledOnce();
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(moreActions());
    vi.useRealTimers();
  });

  it('clears history via a held Space key, and cancels a keyboard hold on Escape', async () => {
    vi.useFakeTimers();
    const onClear = vi.fn();
    await render({ onClear });

    await act(async () => moreActions().click());
    const clear = buttons().find((b) => b.textContent?.includes('Hold to clear history'))!;

    // A held Space key commits after the full duration, same as a pointer hold.
    await act(async () => {
      clear.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }));
    });
    await act(async () => {
      vi.advanceTimersByTime(1200);
    });
    expect(onClear).toHaveBeenCalledOnce();

    // The completed hold above closed the menu; reopen it to hold again and
    // interrupt this one with Escape partway through.
    await act(async () => moreActions().click());
    const clearAgain = buttons().find((b) => b.textContent?.includes('Hold to clear history'))!;
    await act(async () => {
      clearAgain.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }));
    });
    await act(async () => {
      vi.advanceTimersByTime(300);
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(onClear).toHaveBeenCalledOnce();
    vi.useRealTimers();
  });

  it('focuses the search box when the focus token changes', async () => {
    await render();
    await render({ focusSearchToken: 1 });
    expect(document.activeElement).toBe(searchInput());
    expect(searchShell().dataset.expanded).toBe('true');
  });

  it('keeps search visible and keyboard reachable', async () => {
    await render();
    expect(searchShell().dataset.expanded).toBe('true');
    expect(searchInput().tabIndex).toBe(0);
  });

  it('keeps a non-empty query visible', async () => {
    await render();
    await type('tauri');

    expect(searchShell().dataset.expanded).toBe('true');
    expect(searchInput().value).toBe('tauri');
    expect(cardText()).toHaveLength(1);
  });

  it('Escape clears the current search without hiding the control', async () => {
    await render();
    await type('tauri');

    await act(async () => {
      searchInput().dispatchEvent(new KeyboardEvent('keydown', {
        key: 'Escape',
        bubbles: true,
      }));
    });
    expect(searchInput().value).toBe('');
    expect(searchShell().dataset.expanded).toBe('true');
    expect(cardText()).toHaveLength(3);
  });

  it('the clear control resets the active search', async () => {
    await render();
    await type('tauri');
    await act(async () => searchClose().click());

    expect(searchInput().value).toBe('');
    expect(searchShell().dataset.expanded).toBe('true');
    expect(cardText()).toHaveLength(3);
  });

  it('uses an actions group and restores trigger focus on Escape', async () => {
    await render();
    const trigger = moreActions();
    await act(async () => trigger.click());
    const firstAction = document.querySelector(
      '[aria-label="Copy 3 shown as Markdown"]',
    ) as HTMLElement;
    firstAction.focus();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    });
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it('offers file transcription from the overflow menu', async () => {
    const onTranscribeFile = vi.fn();
    await render({ onTranscribeFile });
    await act(async () => moreActions().click());
    await act(async () => byText('Transcribe audio file…')!.click());
    expect(onTranscribeFile).toHaveBeenCalledOnce();
  });

  it('omits secondary row metadata and mode controls', async () => {
    await render({ entries: [entry({ id: 'raw', text: 'Delivered text.', rawText: 'um delivered text' })] });
    expect(container.querySelector('[data-testid="transcript-counts"]')).toBeNull();
    expect(container.textContent).not.toContain('Apply mode');
    expect(container.querySelector('.transcript-copy')).toBeNull();
  });
});
