import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HistoryEntry } from '../../lib/history';
import type { VocabularyEntry } from '../../lib/settings';
import { DISMISSED_SUGGESTIONS_KEY } from '../../lib/vocabularySuggestions';
import { VocabularyAliasesEditor } from './VocabularyAliasesEditor';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock('../../lib/dictation', () => ({ previewVocabularyAliases: vi.fn() }));

const mocks = vi.hoisted(() => ({ history: [] as HistoryEntry[], loads: 0 }));
vi.mock('../../lib/history', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../lib/history')>()),
  loadHistory: () => { mocks.loads += 1; return mocks.history; },
}));

function entry(id: string, text: string): HistoryEntry {
  return { id, text, timestamp: Number(id), duration: 1 };
}

describe('Vocabulary suggestions in the aliases editor', () => {
  let container: HTMLDivElement;
  let root: Root;
  let latestEntries: VocabularyEntry[] | null;

  function Harness({ initialEnabled = false }: { initialEnabled?: boolean }) {
    const [entries, setEntries] = useState<VocabularyEntry[]>([]);
    const [enabled, setEnabled] = useState(initialEnabled);
    return (
      <VocabularyAliasesEditor
        entries={entries}
        onChange={(next) => { latestEntries = next; setEntries(next); }}
        suggestionsEnabled={enabled}
        onSuggestionsEnabledChange={setEnabled}
      />
    );
  }

  const card = () => container.querySelector('[data-setting-target="vocabulary-suggestions"]') as HTMLElement;
  const button = (name: string) =>
    Array.from(container.querySelectorAll('button')).find((node) =>
      (node.getAttribute('aria-label') ?? node.textContent?.trim()) === name) as HTMLButtonElement | undefined;
  const suggested = () =>
    Array.from(card().querySelectorAll('li p:first-child')).map((node) => node.textContent);

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    latestEntries = null;
    mocks.loads = 0;
    mocks.history = [
      entry('1', 'Ask Priya about the GitHub branch.'),
      entry('2', 'Then Priya merged the GitHub fix.'),
      entry('3', 'nothing to see here'),
    ];
    localStorage.clear();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    localStorage.clear();
  });

  it('is off by default and does not read history until enabled', async () => {
    await act(async () => root.render(<Harness />));
    const toggle = card().querySelector('[role="switch"]') as HTMLButtonElement;
    expect(toggle.getAttribute('aria-checked')).toBe('false');
    expect(card().querySelector('ul')).toBeNull();
    expect(mocks.loads).toBe(0);

    await act(async () => toggle.click());
    expect(mocks.loads).toBe(1);
    expect(suggested()).toEqual(['GitHub', 'Priya']);
    expect(latestEntries).toBeNull();
  });

  it('adds a suggestion as a global saved spelling and stops suggesting it', async () => {
    await act(async () => root.render(<Harness initialEnabled />));
    await act(async () => button('Add Priya to saved spellings')!.click());
    expect(latestEntries).toEqual([
      expect.objectContaining({ written: 'Priya', aliases: [], enabled: true, scope: { kind: 'global' } }),
    ]);
    expect(suggested()).toEqual(['GitHub']);
    expect(container.querySelector('[aria-live="polite"]')?.textContent).toBe('Added Priya to saved spellings.');
  });

  it('dismisses durably and restores dismissed suggestions', async () => {
    await act(async () => root.render(<Harness initialEnabled />));
    await act(async () => button('Dismiss GitHub')!.click());
    expect(suggested()).toEqual(['Priya']);
    expect(JSON.parse(localStorage.getItem(DISMISSED_SUGGESTIONS_KEY)!)).toEqual(['github']);
    expect(latestEntries).toBeNull();

    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<Harness initialEnabled />));
    expect(suggested()).toEqual(['Priya']);

    await act(async () => button('Restore 1 dismissed')!.click());
    expect(suggested()).toEqual(['GitHub', 'Priya']);
    expect(localStorage.getItem(DISMISSED_SUGGESTIONS_KEY)).toBeNull();
  });

  it('explains empty states', async () => {
    mocks.history = [];
    await act(async () => root.render(<Harness initialEnabled />));
    expect(card().textContent).toContain('No saved transcripts yet.');
    await act(async () => root.unmount());

    mocks.history = [entry('1', 'just one Priya mention')];
    root = createRoot(container);
    await act(async () => root.render(<Harness initialEnabled />));
    expect(card().textContent).toContain('appears in 2 or more saved transcripts');
  });

  it('is hidden when the host does not opt the editor into suggestions', async () => {
    await act(async () => root.render(<VocabularyAliasesEditor entries={[]} onChange={() => {}} />));
    expect(card()).toBeNull();
    expect(mocks.loads).toBe(0);
  });
});
