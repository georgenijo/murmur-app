import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HistoryPanel } from './HistoryPanel';
import { MAX_PINNED_ENTRIES, type HistoryEntry } from '../../lib/history';

const invoke = vi.fn().mockResolvedValue(0);
const save = vi.fn().mockResolvedValue('/Users/me/Documents/murmur-history.md');

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: (...args: unknown[]) => save(...args) }));

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
  entry({ id: 'pinned', text: 'remember the invariant', pinned: true }),
];

describe('HistoryPanel', () => {
  let container: HTMLDivElement;
  let root: Root;
  let writeText: ReturnType<typeof vi.fn>;

  const buttons = () => Array.from(container.querySelectorAll('button'));
  const byText = (text: string) => buttons().find((b) => b.textContent === text);
  const cardText = () => Array.from(container.querySelectorAll('article')).map((a) => a.textContent ?? '');

  async function render(props: Partial<Parameters<typeof HistoryPanel>[0]> = {}) {
    await act(async () => {
      root.render(
        <HistoryPanel
          entries={ENTRIES}
          onClearUnpinned={vi.fn()}
          onClearAll={vi.fn()}
          onUpdateEntry={vi.fn()}
          onTogglePin={vi.fn()}
          {...props}
        />,
      );
    });
  }

  async function type(value: string) {
    const input = container.querySelector('input[type="search"]') as HTMLInputElement;
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
  });

  it('keeps the empty state when there is no history', async () => {
    await render({ entries: [] });
    expect(container.textContent).toContain('No transcription history yet');
    expect(container.querySelector('input[type="search"]')).toBeNull();
  });

  it('orders pinned entries first and badges them', async () => {
    await render();
    expect(cardText()[0]).toContain('remember the invariant');
    expect(cardText()[0]).toContain('Pinned');
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

  it('filters by source and by pinned', async () => {
    await render();
    await act(async () => byText('File')!.click());
    expect(cardText()).toHaveLength(1);
    expect(cardText()[0]).toContain('standup.wav');

    await act(async () => byText('Pinned 1')!.click());
    expect(cardText()).toHaveLength(1);
    expect(cardText()[0]).toContain('remember the invariant');
  });

  it('toggles a pin through the callback', async () => {
    const onTogglePin = vi.fn();
    await render({ onTogglePin });
    const pin = container.querySelector('[aria-label^="Pin transcription"]') as HTMLButtonElement;
    await act(async () => pin.click());
    expect(onTogglePin).toHaveBeenCalledOnce();
  });

  it('explains the pin ceiling instead of silently dropping the request', async () => {
    const onTogglePin = vi.fn();
    const full = [
      ...Array.from({ length: MAX_PINNED_ENTRIES }, (_, i) => entry({ id: `p${i}`, pinned: true })),
      entry({ id: 'extra', text: 'one more' }),
    ];
    await render({ entries: full, onTogglePin });
    const pin = container.querySelector('[aria-label^="Pin transcription"]') as HTMLButtonElement;
    await act(async () => pin.click());
    expect(onTogglePin).not.toHaveBeenCalled();
    expect(container.textContent).toContain('Pin limit reached');
  });

  it('copies only the visible entries as markdown', async () => {
    await render();
    await type('tauri');
    await act(async () => byText('Export ▾')!.click());
    const copyMarkdown = Array.from(container.querySelectorAll('[role="menuitem"]'))
      .find((item) => item.textContent === 'Markdown') as HTMLButtonElement;
    await act(async () => copyMarkdown.click());
    const calls = writeText.mock.calls;
    const written = calls[calls.length - 1][0] as string;
    expect(written).toContain('# Murmur transcript history');
    expect(written).toContain('ship the Tauri release notes');
    expect(written).not.toContain('imported meeting audio');
    expect(container.textContent).toContain('Copied 1 entry');
  });

  it('saves an export through the native dialog and the validated command', async () => {
    await render();
    await act(async () => byText('Export ▾')!.click());
    const saveJson = Array.from(container.querySelectorAll('[role="menuitem"]'))
      .find((item) => item.textContent === 'JSON…') as HTMLButtonElement;
    await act(async () => saveJson.click());
    expect(save).toHaveBeenCalledOnce();
    expect(save.mock.calls[0][0].defaultPath).toMatch(/^murmur-history-.*\.json$/);
    const [command, payload] = invoke.mock.calls[invoke.mock.calls.length - 1];
    expect(command).toBe('save_text_export');
    expect(JSON.parse((payload as { contents: string }).contents).schema).toBe('murmur.history.v1');
    expect(container.textContent).toContain('Saved 3 entries');
  });

  it('says nothing when the save dialog is cancelled', async () => {
    save.mockResolvedValueOnce(null);
    await render();
    await act(async () => byText('Export ▾')!.click());
    const saveMarkdown = Array.from(container.querySelectorAll('[role="menuitem"]'))
      .find((item) => item.textContent === 'Markdown…') as HTMLButtonElement;
    await act(async () => saveMarkdown.click());
    expect(invoke.mock.calls.filter(([name]) => name === 'save_text_export')).toHaveLength(0);
    expect(container.textContent).not.toContain('Saved');
  });

  it('offers a pinned-safe clear and a separate clear-all', async () => {
    const onClearUnpinned = vi.fn();
    const onClearAll = vi.fn();
    await render({ onClearUnpinned, onClearAll });

    const clear = byText('Clear 2 unpinned')!;
    await act(async () => clear.click());
    await act(async () => clear.click());
    expect(onClearUnpinned).toHaveBeenCalledOnce();
    expect(onClearAll).not.toHaveBeenCalled();

    const clearAll = byText('Clear all')!;
    await act(async () => clearAll.click());
    await act(async () => clearAll.click());
    expect(onClearAll).toHaveBeenCalledOnce();
  });

  it('focuses the search box when the focus token changes', async () => {
    await render();
    await render({ focusSearchToken: 1 });
    expect(document.activeElement).toBe(container.querySelector('input[type="search"]'));
  });
});
