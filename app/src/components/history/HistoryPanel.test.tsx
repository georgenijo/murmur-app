import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HistoryPanel } from './HistoryPanel';
import { type HistoryEntry } from '../../lib/history';

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
  entry({ id: 'note', text: 'remember the invariant', timestamp: Date.UTC(2026, 6, 18, 13) }),
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
          onClear={vi.fn()}
          onUpdateEntry={vi.fn()}
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

  it('orders entries newest first', async () => {
    await render();
    expect(cardText()[0]).toContain('remember the invariant');
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
    await act(async () => byText('File')!.click());
    expect(cardText()).toHaveLength(1);
    expect(cardText()[0]).toContain('standup.wav');
  });

  it('copies only the visible entries as markdown', async () => {
    await render();
    await type('tauri');
    await act(async () => byText('Export ▾')!.click());
    const copyMarkdown = container.querySelector(
      'button[aria-label="Copy 1 shown as Markdown"]',
    ) as HTMLButtonElement;
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
    const saveJson = container.querySelector(
      'button[aria-label="Save 3 shown as JSON"]',
    ) as HTMLButtonElement;
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
    const saveMarkdown = container.querySelector(
      'button[aria-label="Save 3 shown as Markdown"]',
    ) as HTMLButtonElement;
    await act(async () => saveMarkdown.click());
    expect(invoke.mock.calls.filter(([name]) => name === 'save_text_export')).toHaveLength(0);
    expect(container.textContent).not.toContain('Saved');
  });

  it('clears only after a second confirming click', async () => {
    const onClear = vi.fn();
    await render({ onClear });

    const clear = byText('Clear History')!;
    await act(async () => clear.click());
    expect(onClear).not.toHaveBeenCalled();
    expect(container.textContent).toContain('Click again to confirm');
    await act(async () => clear.click());
    expect(onClear).toHaveBeenCalledOnce();
  });

  it('focuses the search box when the focus token changes', async () => {
    await render();
    await render({ focusSearchToken: 1 });
    expect(document.activeElement).toBe(container.querySelector('input[type="search"]'));
  });

  it('uses a disclosure group and restores Export focus on Escape', async () => {
    await render();
    const trigger = byText('Export ▾')!;
    await act(async () => trigger.click());
    const firstAction = container.querySelector(
      'button[aria-label="Copy 3 shown as Markdown"]',
    ) as HTMLButtonElement;
    firstAction.focus();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    });
    expect(container.querySelector('[aria-label="History export actions"]')).toBeNull();
    expect(container.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });
});
