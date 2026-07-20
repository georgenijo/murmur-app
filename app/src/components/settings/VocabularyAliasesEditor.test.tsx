import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { VocabularyEntry } from '../../lib/settings';
import { VocabularyAliasesEditor } from './VocabularyAliasesEditor';

vi.mock('../../lib/dictation', () => ({
  previewVocabularyAliases: vi.fn(async (_entries, _commands, text: string) => text),
}));

const TAURI_ENTRY: VocabularyEntry = {
  id: 'tauri',
  written: 'Tauri',
  aliases: ['Tori', 'Tory'],
  enabled: true,
  scope: { kind: 'global' },
};

describe('VocabularyAliasesEditor', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onChange = vi.fn();

  beforeEach(async () => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    onChange.mockReset();
    await act(async () => root.render(
      <VocabularyAliasesEditor entries={[TAURI_ENTRY]} voiceCommands={[]} onChange={onChange} />,
    ));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('supports disable, delete, and add with inline validation', async () => {
    const toggle = container.querySelector('[role="switch"]') as HTMLButtonElement;
    await act(async () => toggle.click());
    expect(onChange).toHaveBeenLastCalledWith([{ ...TAURI_ENTRY, enabled: false }]);

    const deleteButton = container.querySelector('[aria-label="Delete Tauri"]') as HTMLButtonElement;
    await act(async () => deleteButton.click());
    expect(onChange).toHaveBeenLastCalledWith([]);

    const addButton = Array.from(container.querySelectorAll('button'))
      .find((button) => button.textContent === 'Add term') as HTMLButtonElement;
    await act(async () => addButton.click());
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('written form');
  });

  it('shows the canonical term, aliases, global scope, and local preview affordance', () => {
    expect((container.querySelector('[aria-label="Written form 1"]') as HTMLInputElement).value).toBe('Tauri');
    expect((container.querySelector('[aria-label="Spoken aliases for Tauri"]') as HTMLInputElement).value).toBe('Tori, Tory');
    expect(container.textContent).toContain('Global');
    expect(container.textContent).toContain('Try your aliases');
    expect(container.textContent).toContain('Nothing is recorded, copied, or logged.');
  });
});
