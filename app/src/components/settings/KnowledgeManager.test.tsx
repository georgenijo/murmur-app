import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { KnowledgeEntry } from '../../lib/knowledge';
import { KnowledgeManager } from './KnowledgeManager';

const mocks = vi.hoisted(() => ({
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
    }]} />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('shows searchable scoped records and supports edit, disable, and delete', async () => {
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
});
