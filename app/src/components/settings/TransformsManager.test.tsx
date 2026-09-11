import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { KnowledgeDraft, KnowledgeEntry, KnowledgeListRequest } from '../../lib/knowledge';
import { TransformsManager } from './TransformsManager';

const source: KnowledgeEntry = {
  id: 'original', payload: { kind: 'transform', name: 'Meeting notes', instruction: 'Summarize action items.' },
  enabled: false, scope: { kind: 'global' }, provenance: 'manual',
  createdAtMs: 1, updatedAtMs: 2, revision: 3,
};
const mocks = vi.hoisted(() => ({
  entries: [] as KnowledgeEntry[],
  refresh: vi.fn(async () => {}),
  upsert: vi.fn(async (_draft: KnowledgeDraft) => {}),
  list: vi.fn(async (_request: KnowledgeListRequest) => ({ entries: [] as KnowledgeEntry[], total: 0, nextOffset: null as number | null, storeRevision: 3 })),
  toggle: vi.fn(async () => {}), remove: vi.fn(async () => {}),
}));
vi.mock('../../lib/hooks/useKnowledge', () => ({
  useKnowledge: () => ({ entries: mocks.entries, loading: false, error: null, refresh: mocks.refresh }),
}));
vi.mock('../../lib/knowledge', () => ({
  upsertKnowledge: mocks.upsert, listKnowledge: mocks.list,
  setKnowledgeEnabled: mocks.toggle, deleteKnowledge: mocks.remove,
}));
function button(container: HTMLElement, text: string) {
  const result = Array.from(container.querySelectorAll('button')).find((candidate) => candidate.textContent?.trim() === text);
  if (!result) throw new Error(`Missing button: ${text}`);
  return result;
}
function nameInput(container: HTMLElement) {
  const result = container.querySelector('input[aria-label="Spoken name"]');
  if (!(result instanceof HTMLInputElement)) throw new Error('Missing spoken name');
  return result;
}
function rename(container: HTMLElement, value: string) {
  const element = nameInput(container);
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(element, value);
  element.dispatchEvent(new Event('input', { bubbles: true }));
}
describe('TransformsManager', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(async () => {
    vi.resetAllMocks();
    mocks.entries = [structuredClone(source)];
    mocks.list.mockResolvedValue({ entries: mocks.entries, total: 1, nextOffset: null, storeRevision: 3 });
    container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
    await act(async () => root.render(<TransformsManager active />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

  it('creates a prefilled copy without update identity and keeps both entries independent', async () => {
    await act(async () => button(container, 'Duplicate').click());
    expect(nameInput(container).value).toBe('Copy of Meeting notes');
    expect(container.querySelector('textarea')?.value).toBe('Summarize action items.');
    expect(container.querySelector<HTMLInputElement>('input[type="checkbox"]')?.checked).toBe(false);
    expect(mocks.upsert).not.toHaveBeenCalled();
    await act(async () => rename(container, 'Weekly notes'));
    mocks.upsert.mockImplementationOnce(async (draft) => {
      mocks.entries = [...mocks.entries, { ...draft, id: 'copy', revision: 1, provenance: 'manual', createdAtMs: 4, updatedAtMs: 4 }];
    });
    await act(async () => button(container, 'Save').click());
    expect(mocks.upsert).toHaveBeenCalledWith({
      payload: { kind: 'transform', name: 'Weekly notes', instruction: 'Summarize action items.' },
      enabled: false, scope: { kind: 'global' },
    });
    expect(mocks.entries[0]).toEqual(source);
    expect(container.querySelectorAll('li')).toHaveLength(2);
    const copy = container.querySelectorAll('li')[1];
    await act(async () => button(copy, 'Edit').click());
    await act(async () => button(container, 'Save').click());
    expect(mocks.upsert).toHaveBeenLastCalledWith(expect.objectContaining({ id: 'copy', expectedRevision: 1 }));
    await act(async () => button(copy, 'Delete').click());
    await act(async () => button(copy, 'Confirm delete').click());
    expect(mocks.remove).toHaveBeenCalledWith(mocks.entries[1]);
    expect(mocks.entries[0]).toEqual(source);
  });
  it('avoids normalized copy names on later pages, including disabled entries', async () => {
    mocks.list.mockResolvedValueOnce({ entries: [source], total: 3, nextOffset: 1, storeRevision: 3 });
    mocks.list.mockResolvedValueOnce({
      entries: ['COPY of Meeting notes!', 'Copy of meeting notes 2.'].map((name, index) => ({
        ...source, id: `copy-${index}`, payload: { kind: 'transform', name, instruction: 'Keep me.' },
      })), total: 3, nextOffset: null, storeRevision: 3,
    });
    await act(async () => button(container, 'Duplicate').click());
    expect(mocks.list).toHaveBeenLastCalledWith({ kind: 'transform', limit: 100, offset: 1 });
    expect(nameInput(container).value).toBe('Copy of Meeting notes 3');
  });
  it('retains preset warnings and backend conflict validation', async () => {
    await act(async () => button(container, 'Duplicate').click());
    await act(async () => rename(container, 'Shorten!'));
    expect(container.textContent).toContain('This name matches a built-in preset.');
    mocks.upsert.mockRejectedValueOnce(new Error('A saved transform with that name already exists.'));
    await act(async () => button(container, 'Save').click());
    expect(container.textContent).toContain('A saved transform with that name already exists.');
    expect(nameInput(container).value).toBe('Shorten!');
    expect(mocks.refresh).not.toHaveBeenCalled();
    expect(mocks.entries).toEqual([source]);
  });
  it('resets state when switching editors and preserves edit, add, and enable behavior', async () => {
    await act(async () => button(container, 'Edit').click());
    await act(async () => rename(container, 'Unsaved edit'));
    await act(async () => button(container, 'Duplicate').click());
    expect(nameInput(container).value).toBe('Copy of Meeting notes');
    await act(async () => button(container, 'Edit').click());
    expect(nameInput(container).value).toBe('Meeting notes');
    await act(async () => button(container, 'Save').click());
    expect(mocks.upsert).toHaveBeenCalledWith(expect.objectContaining({ id: source.id, expectedRevision: source.revision }));
    await act(async () => button(container, 'Add').click());
    expect(nameInput(container).value).toBe(''); expect(container.querySelector('textarea')?.value).toBe('');
    await act(async () => button(container, 'Cancel').click());
    await act(async () => button(container, 'Enable').click());
    expect(mocks.toggle).toHaveBeenCalledWith(source, true);
  });
  it('reopens the same source as a fresh duplicate and discards previous draft edits', async () => {
    await act(async () => button(container, 'Duplicate').click());
    await act(async () => rename(container, 'Discard this draft'));
    const checkbox = container.querySelector<HTMLInputElement>('input[type="checkbox"]');
    await act(async () => checkbox?.click());
    await act(async () => button(container, 'Duplicate').click());
    expect(nameInput(container).value).toBe('Copy of Meeting notes');
    expect(container.querySelector('textarea')?.value).toBe('Summarize action items.');
    expect(container.querySelector<HTMLInputElement>('input[type="checkbox"]')?.checked).toBe(false);
  });
  it('keeps copy names within the stores Unicode character limit', async () => {
    mocks.entries = [{ ...source, payload: { kind: 'transform', name: '𐐀'.repeat(256), instruction: 'Rewrite.' } }];
    await act(async () => root.render(<TransformsManager active />));
    await act(async () => button(container, 'Duplicate').click());
    expect(Array.from(nameInput(container).value)).toHaveLength(256);
    expect(nameInput(container).value).toMatch(/^Copy of /);
  });
  it('reports name lookup failures without creating a draft', async () => {
    mocks.list.mockRejectedValueOnce(new Error('Store unavailable'));
    await act(async () => button(container, 'Duplicate').click());
    expect(container.textContent).toContain('Store unavailable');
    expect(container.querySelector('textarea')).toBeNull(); expect(mocks.upsert).not.toHaveBeenCalled();
    expect(mocks.entries).toEqual([source]);
  });
});
