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

  const buttons = () => Array.from(document.querySelectorAll<HTMLElement>('button, [role="menuitem"], [role="menuitemradio"], [role="menuitemcheckbox"]'));
  const byText = (text: string) => buttons().find((b) => b.textContent === text);
  const cardText = () => Array.from(container.querySelectorAll('article')).map((a) => a.textContent ?? '');
  const searchShell = () => container.querySelector('[data-testid="history-search-shell"]') as HTMLDivElement;
  const searchInput = () => container.querySelector('input[type="search"]') as HTMLInputElement;
  const searchClose = () => container.querySelector('[aria-label="Clear transcript search"]') as HTMLButtonElement;
  const moreActions = () => container.querySelector('[aria-label="More history actions"]') as HTMLButtonElement;
  const deleteShown = () => document.querySelector('[aria-label^="Hold to delete"]') as HTMLButtonElement | null;
  const filterTrigger = () => container.querySelector('[aria-label^="Filter transcripts"]') as HTMLButtonElement;
  const filterOption = (label: string) => Array.from(document.querySelectorAll<HTMLElement>('[role="menuitemradio"], [role="menuitemcheckbox"]'))
    .find((item) => item.textContent === label);
  // The filter menu stays open between choices so several can be combined.
  async function choose(label: string) {
    if (!filterOption(label)) await act(async () => filterTrigger().click());
    await act(async () => filterOption(label)!.click());
  }

  async function render(props: Partial<Parameters<typeof HistoryPanel>[0]> = {}) {
    await act(async () => {
      root.render(
        <HistoryPanel
          entries={ENTRIES}
          onClear={vi.fn()}
          onDeleteEntries={vi.fn()}
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

  async function quickClick(button: HTMLButtonElement) {
    await act(async () => {
      button.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerId: 1 }));
    });
    await act(async () => {
      vi.advanceTimersByTime(50);
      button.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1 }));
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
    expect(container.textContent).not.toMatch(/\d+ entr/);
    expect(container.querySelector('.history-filtered-note')).toBeNull();
  });

  it('consumes a teach request once and explains when history is empty', async () => {
    const onHandled = vi.fn();
    await render({ entries: [], teachLatestToken: 1, onTeachLatestHandled: onHandled });
    expect(container.textContent).toContain('Make a dictation first');
    expect(onHandled).toHaveBeenCalledOnce();

    await render({ entries: [entry({ id: 'later' })], teachLatestToken: undefined, onTeachLatestHandled: onHandled });
    expect(document.querySelector('[aria-label="Close Correct and Teach"]')).toBeNull();
  });

  it('does not replay a consumed teach request after close, history updates, or remount', async () => {
    const onHandled = vi.fn();
    await render({ teachLatestToken: 2, onTeachLatestHandled: onHandled });
    const close = document.querySelector('[aria-label="Close Correct and Teach"]') as HTMLButtonElement;
    expect(close).not.toBeNull();
    await act(async () => close.click());
    await render({ entries: [...ENTRIES, entry({ id: 'newer', timestamp: Date.UTC(2026, 6, 18, 14) })], teachLatestToken: undefined, onTeachLatestHandled: onHandled });
    expect(document.querySelector('[aria-label="Close Correct and Teach"]')).toBeNull();

    await act(async () => root.unmount());
    root = createRoot(container);
    await render({ teachLatestToken: undefined, onTeachLatestHandled: onHandled });
    expect(document.querySelector('[aria-label="Close Correct and Teach"]')).toBeNull();
    expect(onHandled).toHaveBeenCalledOnce();
  });

  it('opens teaching for each explicit request even when the parent reuses token one', async () => {
    const onHandled = vi.fn();
    await render({ teachLatestToken: 1, onTeachLatestHandled: onHandled });
    await act(async () => (document.querySelector('[aria-label="Close Correct and Teach"]') as HTMLButtonElement).click());
    await render({ teachLatestToken: undefined, onTeachLatestHandled: onHandled });
    await render({ teachLatestToken: 1, onTeachLatestHandled: onHandled });
    expect(document.querySelector('[aria-label="Close Correct and Teach"]')).not.toBeNull();
    expect(onHandled).toHaveBeenCalledTimes(2);
  });

  it('orders entries newest first', async () => {
    await render();
    expect(cardText()[0]).toContain('remember the invariant');
    expect(container.querySelector('.history-filtered-note')).toBeNull();
    expect(container.textContent).not.toContain('3 entries');
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
    expect(container.querySelector('.history-filtered-note')?.textContent).toBe('Showing 1 of 3Show all');
    await act(async () => byText('Show all')!.click());
    expect(cardText()).toHaveLength(3);
    expect(document.activeElement).toBe(searchInput());
  });

  it('does not count a blank search as filtering', async () => {
    await render();
    await type('   ');
    expect(cardText()).toHaveLength(3);
    expect(container.querySelector('.history-filtered-note')).toBeNull();
  });

  it('refreshes date-filtered results when the window regains focus', async () => {
    const firstDay = new Date(2026, 6, 18, 12).getTime();
    const nextDay = new Date(2026, 6, 19, 12).getTime();
    vi.spyOn(Date, 'now').mockReturnValue(firstDay);
    await render({ entries: [entry({ id: 'dated', timestamp: firstDay })] });
    await choose('Today');
    expect(cardText()).toHaveLength(1);
    vi.spyOn(Date, 'now').mockReturnValue(nextDay);
    await act(async () => window.dispatchEvent(new Event('focus')));
    expect(cardText()).toHaveLength(0);
    expect(container.textContent).toContain('No matching transcripts');
  });

  it('includes new transcripts without waiting for focus or midnight', async () => {
    const start = new Date(2026, 6, 18, 12).getTime();
    const clock = vi.spyOn(Date, 'now').mockReturnValue(start);
    const first = entry({ id: 'first', timestamp: start, text: 'first transcript' });
    await render({ entries: [first] });
    await choose('Today');
    clock.mockReturnValue(start + 60_000);
    await render({ entries: [first, entry({
      id: 'new', timestamp: start + 60_000, text: 'new transcript',
    })] });
    expect(cardText()).toHaveLength(2);
    expect(cardText()[0]).toContain('new transcript');
  });

  it('shows an empty-result state that resets the filters', async () => {
    await render();
    await choose('Today');
    await act(async () => filterTrigger().click());
    await type('nothing matches this');
    expect(container.textContent).toContain('No matching transcripts');
    await act(async () => byText('Show all')!.click());
    expect(cardText()).toHaveLength(3);
    expect(searchInput().value).toBe('');
    expect(filterTrigger().getAttribute('aria-label')).toBe('Filter transcripts');
    expect(container.querySelector('.history-filtered-note')).toBeNull();
  });

  it('filters by source from one menu and names the active filters on the trigger', async () => {
    await render();
    expect(filterTrigger().textContent).toBe('Filter');
    await act(async () => filterTrigger().click());
    expect(filterOption('Everything')!.getAttribute('aria-checked')).toBe('true');
    expect(filterOption('Any time')!.getAttribute('aria-checked')).toBe('true');
    await choose('Files');
    expect(filterOption('Everything')!.getAttribute('aria-checked')).toBe('false');
    expect(filterOption('Files')!.getAttribute('aria-checked')).toBe('true');
    expect(cardText()).toHaveLength(1);
    expect(cardText()[0]).toContain('standup.wav');
    expect(filterTrigger().textContent).toBe('Files');
    await choose('Today');
    expect(filterTrigger().textContent).toBe('2 filters');
    expect(filterTrigger().getAttribute('aria-label')).toBe('Filter transcripts: 2 filters, Files · Today');
    await act(async () => byText('Clear filters')!.click());
    expect(filterTrigger().textContent).toBe('Filter');
    expect(cardText()).toHaveLength(3);
  });

  it('tags only file entries, not every spoken one', async () => {
    await render();
    const spoken = Array.from(container.querySelectorAll('article')).find((card) => card.textContent?.includes('ship the Tauri'))!;
    expect(spoken.querySelector('.transcript-meta')?.textContent).not.toContain('Mic');
    expect(cardText().find((text) => text.includes('imported meeting audio'))).toContain('standup.wav');
  });

  it('pins an entry and filters pinned results without changing export scope', async () => {
    const onTogglePinned = vi.fn();
    await render({
      entries: [
        entry({ id: 'pinned', text: 'keep this snippet', pinned: true }),
        entry({ id: 'other', text: 'temporary note' }),
      ],
      onTogglePinned,
      pinnedCount: 1,
    });
    await choose('Pinned only');
    expect(cardText()).toHaveLength(1);
    expect(cardText()[0]).toContain('keep this snippet');
    expect(filterOption('Pinned only')!.getAttribute('aria-checked')).toBe('true');
    await choose('Pinned only');
    expect(filterOption('Pinned only')!.getAttribute('aria-checked')).toBe('false');
    expect(cardText()).toHaveLength(2);
    await act(async () => filterTrigger().click());
    const otherCard = Array.from(container.querySelectorAll('article')).find((card) => card.textContent?.includes('temporary note'))!;
    await act(async () => (otherCard.querySelector('[aria-label="More transcript actions"]') as HTMLElement).click());
    await act(async () => byText('Pin transcript')!.click());
    expect(onTogglePinned).toHaveBeenCalledWith(expect.objectContaining({ id: 'other' }));
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

  it('offers Delete shown only while search or filters narrow history', async () => {
    await render();
    await act(async () => moreActions().click());
    expect(deleteShown()).toBeNull();

    await act(async () => moreActions().click());
    await type('tauri');
    await act(async () => moreActions().click());
    expect(deleteShown()?.textContent).toContain('Hold to delete 1 shown');
  });

  it('deletes the exact filtered subset, including a pin, without matching duplicate ids', async () => {
    vi.useFakeTimers();
    const matching = entry({ id: 'duplicate', text: 'remove alpha', pinned: true });
    const nonmatching = entry({ id: 'duplicate', text: 'keep beta', pinned: true });
    const onDeleteEntries = vi.fn();
    const onClear = vi.fn();
    await render({ entries: [matching, nonmatching], onDeleteEntries, onClear });
    await type('alpha');
    await act(async () => moreActions().click());

    const button = deleteShown()!;
    await quickClick(button);
    expect(onDeleteEntries).not.toHaveBeenCalled();
    await quickClick(button);

    expect(onDeleteEntries).toHaveBeenCalledOnce();
    expect(onDeleteEntries).toHaveBeenCalledWith([matching]);
    expect(onClear).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('deletes every matching entry beyond the mounted batch', async () => {
    vi.useFakeTimers();
    const matches = Array.from({ length: 35 }, (_, index) => entry({
      id: `match-${index}`,
      text: `matched transcript ${index}`,
      timestamp: Date.UTC(2026, 6, 18, 12, index),
    }));
    const nonmatches = Array.from({ length: 5 }, (_, index) => entry({
      id: `other-${index}`,
      text: `other transcript ${index}`,
    }));
    const onDeleteEntries = vi.fn();
    await render({ entries: [...matches, ...nonmatches], onDeleteEntries });
    await type('matched');
    expect(cardText()).toHaveLength(30);
    await act(async () => moreActions().click());

    const button = deleteShown()!;
    await quickClick(button);
    await quickClick(button);

    expect(onDeleteEntries).toHaveBeenCalledOnce();
    const deleted = onDeleteEntries.mock.calls[0][0] as HistoryEntry[];
    expect(deleted).toHaveLength(35);
    expect(new Set(deleted)).toEqual(new Set(matches));
    vi.useRealTimers();
  });

  it('disarms Delete shown after four seconds', async () => {
    vi.useFakeTimers();
    const onDeleteEntries = vi.fn();
    await render({ onDeleteEntries });
    await type('tauri');
    await act(async () => moreActions().click());

    const button = deleteShown()!;
    await quickClick(button);
    expect(button.textContent).toContain('Click again to delete 1 shown');
    await act(async () => vi.advanceTimersByTime(4000));
    expect(button.textContent).toContain('Hold to delete 1 shown');
    await quickClick(button);

    expect(onDeleteEntries).not.toHaveBeenCalled();
    expect(button.textContent).toContain('Click again to delete 1 shown');
    vi.useRealTimers();
  });

  it('disarms an armed delete when the shown scope changes', async () => {
    vi.useFakeTimers();
    const alpha = entry({ id: 'alpha', text: 'alpha transcript' });
    const beta = entry({ id: 'beta', text: 'beta transcript' });
    const onDeleteEntries = vi.fn();
    await render({ entries: [alpha, beta], onDeleteEntries });
    await type('alpha');
    await act(async () => moreActions().click());
    await quickClick(deleteShown()!);

    await type('beta');
    const changedScopeButton = deleteShown()!;
    expect(changedScopeButton.textContent).toContain('Hold to delete 1 shown');
    await quickClick(changedScopeButton);
    expect(onDeleteEntries).not.toHaveBeenCalled();
    await quickClick(changedScopeButton);

    expect(onDeleteEntries).toHaveBeenCalledWith([beta]);
    vi.useRealTimers();
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
    expect(middleCard.querySelector('[data-action-id="copy"]')?.textContent).toBe('Copied');
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
    await act(async () => byText('Transcribe audio or video file…')!.click());
    expect(onTranscribeFile).toHaveBeenCalledOnce();
  });

  it('omits secondary row metadata and mode controls', async () => {
    await render({ entries: [entry({ id: 'raw', text: 'Delivered text.', rawText: 'um delivered text' })] });
    expect(container.querySelector('[data-testid="transcript-counts"]')).toBeNull();
    expect(container.textContent).not.toContain('Apply mode');
    expect(container.querySelector('.transcript-copy')).toBeNull();
  });
});
