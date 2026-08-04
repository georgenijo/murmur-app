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
      .find((button) => button.textContent?.includes('Add spelling')) as HTMLButtonElement;
    await act(async () => addButton.click());
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container.querySelectorAll('[aria-label^="Written form"]')).toHaveLength(1);
  });

  it('shows a compact hears-to-types mapping and a collapsed local preview', async () => {
    expect((container.querySelector('[aria-label="Written form 1"]') as HTMLInputElement).value).toBe('Tauri');
    expect((container.querySelector('[aria-label="Spoken aliases for Tauri"]') as HTMLInputElement).value).toBe('Tori, Tory');
    expect(container.textContent).toContain('Murmur hears');
    expect(container.textContent).toContain('Murmur types');
    expect(container.textContent).not.toContain('Spoken aliases');
    expect(container.textContent).not.toContain('Global');

    const previewButton = Array.from(container.querySelectorAll('button'))
      .find((button) => button.textContent?.includes('Test a phrase')) as HTMLButtonElement;
    expect(previewButton.getAttribute('aria-expanded')).toBe('false');
    await act(async () => previewButton.click());
    expect(container.textContent).toContain('Nothing is copied or logged.');
    expect(container.querySelector('[aria-label="Alias preview input"]')).not.toBeNull();
  });

  it('caps the visible list height so saved spellings do not grow the settings page forever', () => {
    const list = container.querySelector('[aria-label="Saved spellings"]') as HTMLDivElement;
    expect(list.className).toContain('max-h-[286px]');
    expect(list.className).toContain('overflow-y-auto');
  });
});
