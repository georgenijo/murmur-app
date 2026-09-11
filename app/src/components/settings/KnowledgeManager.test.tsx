import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { KnowledgeEntry, KnowledgeListResponse } from '../../lib/knowledge';
import { KnowledgeManager } from './KnowledgeManager';

const mocks = vi.hoisted(() => ({
  list: vi.fn<() => Promise<KnowledgeListResponse>>(),
  refresh: vi.fn(async () => {}),
  loadMore: vi.fn(async () => {}),
  setStatus: vi.fn(),
  upsert: vi.fn(async () => {}),
  toggle: vi.fn(async () => {}),
  remove: vi.fn(async () => 2),
  removeAll: vi.fn(async () => 3),
  retry: vi.fn(),
  exportFile: vi.fn(async () => 1),
  inspectImport: vi.fn(),
  importFile: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
}));

const ENTRY: KnowledgeEntry = {
  id: 'record-1',
  payload: { kind: 'replacement_rule', source: 'Tory', replacement: 'Tauri' },
  enabled: true,
  scope: { kind: 'app', bundleId: 'com.apple.Terminal' },
  provenance: 'manual',
  createdAtMs: 1_700_000_000_000,
  updatedAtMs: 1_700_000_000_000,
  revision: 4,
};

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: mocks.open, save: mocks.save }));
vi.mock('../../lib/hooks/useKnowledge', () => ({
  useKnowledge: () => ({
    status: { availability: 'ready', schemaVersion: 2, recordCount: 1, storeRevision: 9, recoveryAtMs: null, message: null },
    entries: [ENTRY],
    total: 1,
    nextOffset: null,
    loading: false,
    error: null,
    refresh: mocks.refresh,
    loadMore: mocks.loadMore,
    setStatus: mocks.setStatus,
  }),
}));
vi.mock('../../lib/knowledge', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../lib/knowledge')>(),
  listKnowledge: mocks.list,
  upsertKnowledge: mocks.upsert,
  setKnowledgeEnabled: mocks.toggle,
  deleteKnowledge: mocks.remove,
  deleteAllKnowledge: mocks.removeAll,
  retryKnowledgeStore: mocks.retry,
  exportKnowledgeToFile: mocks.exportFile,
  inspectKnowledgeImport: mocks.inspectImport,
  importKnowledgeFromFile: mocks.importFile,
}));

function button(container: HTMLElement, text: string) {
  return Array.from(container.querySelectorAll('button')).find((candidate) => candidate.textContent?.trim() === text) as HTMLButtonElement;
}

function setValue(element: HTMLInputElement | HTMLSelectElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(element.constructor.prototype, 'value')?.set;
  setter?.call(element, value);
  element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
}

describe('KnowledgeManager', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    vi.clearAllMocks();
    mocks.list.mockReset().mockResolvedValue({ entries: [ENTRY], total: 1, nextOffset: null, storeRevision: 9 });
    mocks.toggle.mockReset().mockResolvedValue();
    mocks.remove.mockReset().mockResolvedValue(2);
    mocks.retry.mockResolvedValue({ availability: 'ready', schemaVersion: 2, recordCount: 1, storeRevision: 9, recoveryAtMs: null, message: null });
    mocks.inspectImport.mockResolvedValue({ total: 2, new: 1, duplicates: 1, conflicts: 0 });
    mocks.importFile.mockResolvedValue({ imported: 1, duplicates: 1, storeRevision: 10 });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<KnowledgeManager active profiles={[{
      bundleId: 'com.apple.Terminal', label: 'Terminal', autoPasteOverride: null,
      cleanupOverride: null, smartFormattingOverride: null, cliFormattingOverride: null,
      writingStyle: null, ideContextEnabled: false, ideProjectRoots: [],
      queryContextExcluded: false,
    }]} />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('shows searchable scoped records and supports edit, disable, delete', async () => {
    expect(container.textContent).toContain('Tory');
    expect(container.textContent).toContain('App · com.apple.Terminal');
    expect(container.querySelector('[aria-label="Search personal knowledge"]')).not.toBeNull();

    await act(async () => (container.querySelector('[role="switch"]') as HTMLButtonElement).click());
    expect(mocks.toggle).toHaveBeenCalledWith(ENTRY, false);

    await act(async () => (container.querySelector('[aria-label="Edit Tory"]') as HTMLButtonElement).click());
    expect(container.querySelector('[role="dialog"]')?.textContent).toContain('Edit knowledge');
    await act(async () => button(container, 'Cancel').click());

    await act(async () => (container.querySelector('[aria-label="Delete Tory"]') as HTMLButtonElement).click());
    expect(container.querySelector('[role="dialog"]')?.textContent).toContain('Delete this knowledge');
    const deleteButtons = Array.from(container.querySelectorAll('button')).filter((candidate) => candidate.textContent?.trim() === 'Delete');
    await act(async () => deleteButtons[deleteButtons.length - 1].click());
    expect(mocks.remove).toHaveBeenCalledWith(ENTRY);
  });

  it('creates each type and exposes explicit visibility controls', async () => {
    await act(async () => button(container, 'Create knowledge').click());
    const type = container.querySelector('[role="dialog"] select') as HTMLSelectElement;
    expect(container.querySelector('[aria-label="Heard phrase"]')).not.toBeNull();
    await act(async () => setValue(type, 'vocabulary_term'));
    expect(container.querySelector('[aria-label="Written form"]')).not.toBeNull();
    await act(async () => setValue(type, 'snippet'));
    expect(container.querySelector('[aria-label="Snippet body"]')).not.toBeNull();
    expect(container.textContent).toContain('One project in one app');
  });

  it('previews import, exports, and requires typed DELETE before delete-all', async () => {
    mocks.save.mockResolvedValue('/tmp/knowledge.json');
    await act(async () => button(container, 'Export…').click());
    expect(mocks.exportFile).toHaveBeenCalledWith('/tmp/knowledge.json');

    mocks.open.mockResolvedValue('/tmp/knowledge.json');
    await act(async () => button(container, 'Import…').click());
    expect(container.querySelector('[role="dialog"]')?.textContent).toContain('2 records inspected');
    await act(async () => button(container, 'Import').click());
    expect(mocks.importFile).toHaveBeenCalledWith('/tmp/knowledge.json');

    await act(async () => button(container, 'Delete all…').click());
    const confirm = button(container, 'Delete everything');
    expect(confirm.disabled).toBe(true);
    const input = container.querySelector('[aria-label="Type DELETE to confirm"]') as HTMLInputElement;
    await act(async () => setValue(input, 'DELETE'));
    expect(button(container, 'Delete everything').disabled).toBe(false);
    await act(async () => button(container, 'Delete everything').click());
    expect(mocks.removeAll).toHaveBeenCalledWith(9);
  });

  it('uses current normalized filters and disables every match beyond the visible page', async () => {
    const matches = Array.from({ length: 55 }, (_, index) => ({ ...ENTRY, id: `match-${index}`, revision: index + 1 }));
    mocks.list.mockResolvedValueOnce({ entries: matches.slice(0, 50), total: 55, nextOffset: 50, storeRevision: 9 });
    mocks.list.mockResolvedValueOnce({ entries: matches.slice(50), total: 55, nextOffset: null, storeRevision: 9 });
    await act(async () => {
      setValue(container.querySelector<HTMLInputElement>('[aria-label="Search personal knowledge"]')!, '  match  ');
      setValue(container.querySelector<HTMLSelectElement>('[aria-label="Filter knowledge type"]')!, 'replacement_rule');
      setValue(container.querySelector<HTMLSelectElement>('[aria-label="Filter enabled state"]')!, 'enabled');
      setValue(container.querySelector<HTMLSelectElement>('[aria-label="Filter visibility"]')!, 'app');
    });
    await act(async () => button(container, 'Disable all shown').click());
    expect(mocks.list.mock.calls).toEqual([
      [{ query: 'match', kind: 'replacement_rule', enabled: true, scopeKind: 'app', limit: 50, offset: 0 }],
      [{ query: 'match', kind: 'replacement_rule', enabled: true, scopeKind: 'app', limit: 50, offset: 50 }],
    ]);
    expect(mocks.toggle.mock.calls).toEqual(matches.map((entry) => [entry, false]));
    expect(mocks.list.mock.invocationCallOrder[1]).toBeLessThan(mocks.toggle.mock.invocationCallOrder[0]);
    expect(container.textContent).toContain('55 records disabled; 0 failed.');
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });

  it('requires typed DELETE for the captured scope and reports each failed record', async () => {
    const second = { ...ENTRY, id: 'record-2', revision: 17 };
    mocks.list.mockResolvedValueOnce({ entries: [ENTRY, second], total: 2, nextOffset: null, storeRevision: 9 });
    await act(async () => setValue(container.querySelector<HTMLInputElement>('[aria-label="Search personal knowledge"]')!, 'Tory'));
    await act(async () => button(container, 'Delete all shown…').click());
    expect(container.querySelector('[role="dialog"]')?.textContent).toContain('search: “Tory”');
    expect(button(container, 'Delete 2 matching records').disabled).toBe(true);
    expect(mocks.remove).not.toHaveBeenCalled();
    expect(container.querySelector<HTMLInputElement>('[aria-label="Search personal knowledge"]')?.disabled).toBe(true);
    // A filter change behind the modal must not change the confirmed snapshot.
    await act(async () => setValue(container.querySelector<HTMLInputElement>('[aria-label="Search personal knowledge"]')!, 'other'));
    await act(async () => setValue(container.querySelector<HTMLInputElement>('[aria-label="Type DELETE to confirm matching records"]')!, 'DELETE'));
    mocks.remove.mockRejectedValueOnce('Record changed; refresh before retrying.').mockResolvedValueOnce(10);
    const confirm = button(container, 'Delete 2 matching records');
    await act(async () => { confirm.click(); confirm.click(); });
    expect(mocks.list).toHaveBeenCalledTimes(1);
    expect(mocks.remove.mock.calls).toEqual([[ENTRY], [second]]);
    expect(mocks.removeAll).not.toHaveBeenCalled();
    expect(container.textContent).toContain('1 records removed; 1 failed.');
    expect(container.textContent).toContain('Tory · record-1: Record changed; refresh before retrying.');
  });

  it('cancels deletion without writes and collects the next scope afresh', async () => {
    await act(async () => button(container, 'Delete all shown…').click());
    await act(async () => setValue(container.querySelector<HTMLInputElement>('[aria-label="Type DELETE to confirm matching records"]')!, 'DELETE'));
    await act(async () => button(container, 'Cancel').click());
    expect(mocks.remove).not.toHaveBeenCalled();
    await act(async () => setValue(container.querySelector<HTMLSelectElement>('[aria-label="Filter visibility"]')!, 'global'));
    await act(async () => button(container, 'Delete all shown…').click());
    expect(button(container, 'Delete 1 matching records').disabled).toBe(true);
    expect(mocks.list).toHaveBeenLastCalledWith(expect.objectContaining({ scopeKind: 'global' }));
  });

  it('locks repeated clicks and refuses store drift without any writes', async () => {
    let resolvePage: ((value: KnowledgeListResponse) => void) | undefined;
    mocks.list.mockImplementationOnce(() => new Promise((resolve) => { resolvePage = resolve; }));
    mocks.list.mockResolvedValueOnce({ entries: [{ ...ENTRY, id: 'second' }], total: 2, nextOffset: null, storeRevision: 10 });
    const enable = button(container, 'Enable all shown');
    await act(async () => { enable.click(); enable.click(); });
    expect(mocks.list).toHaveBeenCalledTimes(1);
    expect(button(container, 'Disable all shown').disabled).toBe(true);
    expect(container.querySelector<HTMLButtonElement>('[role="switch"]')?.disabled).toBe(true);
    await act(async () => resolvePage?.({ entries: [ENTRY], total: 2, nextOffset: 1, storeRevision: 9 }));
    expect(mocks.toggle).not.toHaveBeenCalled();
    expect(container.textContent).toContain('Refresh and try again. No records were changed.');
  });

  it('does not write when deactivated during collection', async () => {
    let resolvePage: ((value: KnowledgeListResponse) => void) | undefined;
    mocks.list.mockImplementationOnce(() => new Promise((resolve) => { resolvePage = resolve; }));
    await act(async () => button(container, 'Enable all shown').click());
    await act(async () => root.render(<KnowledgeManager active={false} profiles={[]} />));
    await act(async () => resolvePage?.({ entries: [ENTRY], total: 1, nextOffset: null, storeRevision: 9 }));
    expect(mocks.toggle).not.toHaveBeenCalled();
    await act(async () => root.render(<KnowledgeManager active profiles={[]} />));
    await act(async () => button(container, 'Disable all shown').click());
    expect(mocks.toggle).toHaveBeenCalledWith(ENTRY, false);
  });

  it('stops further writes when unmounted during a batch', async () => {
    let resolveWrite: (() => void) | undefined;
    mocks.list.mockResolvedValueOnce({ entries: [ENTRY, { ...ENTRY, id: 'second' }], total: 2, nextOffset: null, storeRevision: 9 });
    mocks.toggle.mockImplementationOnce(() => new Promise((resolve) => { resolveWrite = resolve; }));
    await act(async () => button(container, 'Disable all shown').click());
    expect(mocks.toggle).toHaveBeenCalledTimes(1);
    await act(async () => root.render(<div />));
    await act(async () => resolveWrite?.());
    expect(mocks.toggle).toHaveBeenCalledTimes(1);
    expect(mocks.refresh).not.toHaveBeenCalled();
  });
});
