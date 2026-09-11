import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { KnowledgeDraft, KnowledgeEntry } from '../../lib/knowledge';
import { VoiceCommandsManager } from './VoiceCommandsManager';

const mocks = vi.hoisted(() => ({
  refresh: vi.fn(async () => {}),
  loadMore: vi.fn(async () => {}),
  upsert: vi.fn(async (_draft: KnowledgeDraft) => ({})),
  toggle: vi.fn(async () => ({})),
  remove: vi.fn(async () => 2),
  preview: vi.fn(async () => ({ output: 'Yesterday:\n- done', matched: true, clipboardRequired: false, clipboardRead: false })),
}));

const ENTRY: KnowledgeEntry = {
  id: 'voice-1',
  payload: { kind: 'snippet', trigger: 'insert standup', body: 'Yesterday:\n- done' },
  enabled: true,
  scope: { kind: 'global' },
  provenance: 'manual',
  createdAtMs: 1,
  updatedAtMs: 1,
  revision: 1,
  voiceCommand: { commandType: 'snippet', allowClipboardRead: false },
};

const DUPLICATE_SOURCE: KnowledgeEntry = {
  id: 'voice-2',
  payload: { kind: 'snippet', trigger: 'mail signature', body: 'Regards,\nGeorge\n{{clipboard}}' },
  enabled: false,
  scope: { kind: 'app', bundleId: 'com.apple.mail' },
  provenance: 'manual',
  createdAtMs: 2,
  updatedAtMs: 3,
  revision: 4,
  voiceCommand: { commandType: 'snippet', allowClipboardRead: true },
};

vi.mock('../../lib/hooks/useKnowledge', () => ({
  useKnowledge: () => ({
    status: { availability: 'ready', schemaVersion: 3, recordCount: 1, storeRevision: 4, recoveryAtMs: null, message: null },
    entries: [ENTRY, DUPLICATE_SOURCE], total: 2, nextOffset: null, loading: false, error: null,
    refresh: mocks.refresh, loadMore: mocks.loadMore, setStatus: vi.fn(),
  }),
}));

vi.mock('../../lib/knowledge', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../lib/knowledge')>(),
  upsertKnowledge: mocks.upsert,
  setKnowledgeEnabled: mocks.toggle,
  deleteKnowledge: mocks.remove,
  previewVoiceCommand: mocks.preview,
}));

function button(container: HTMLElement, text: string) {
  return Array.from(container.querySelectorAll('button')).find((candidate) => candidate.textContent?.trim() === text) as HTMLButtonElement;
}

function setValue(element: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(element.constructor.prototype, 'value')?.set;
  setter?.call(element, value);
  element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
}

describe('VoiceCommandsManager', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    vi.clearAllMocks();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<VoiceCommandsManager active globallyEnabled profiles={[{
      bundleId: 'com.apple.mail', label: 'Mail', autoPasteOverride: null, cleanupOverride: null,
      smartFormattingOverride: null, cliFormattingOverride: null, writingStyle: null,
      ideContextEnabled: false, ideProjectRoots: [],
      queryContextExcluded: false,
    }]} />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('lists, disables, edits, and requires an explicit delete confirmation', async () => {
    expect(container.textContent).toContain('insert standup');
    await act(async () => (container.querySelector('[role="switch"]') as HTMLButtonElement).click());
    expect(mocks.toggle).toHaveBeenCalledWith(ENTRY, false);
    await act(async () => button(container, 'Edit').click());
    expect(container.querySelector('[role="dialog"]')?.textContent).toContain('Edit Voice Command');
    await act(async () => button(container, 'Cancel').click());
    await act(async () => button(container, 'Delete').click());
    expect(button(container, 'Confirm delete')).not.toBeNull();
    await act(async () => button(container, 'Confirm delete').click());
    expect(mocks.remove).toHaveBeenCalledWith(ENTRY);
  });

  it('creates an app-scoped multiline snippet with explicit clipboard permission and preview', async () => {
    await act(async () => button(container, 'New command').click());
    await act(async () => setValue(container.querySelector('[aria-label="Voice Command type"]') as HTMLSelectElement, 'snippet'));
    await act(async () => setValue(container.querySelector('[aria-label="Voice Command scope"]') as HTMLSelectElement, 'app'));
    await act(async () => setValue(container.querySelector('[aria-label="Voice Command phrase"]') as HTMLInputElement, 'mail signature'));
    await act(async () => setValue(container.querySelector('[aria-label="Voice Command content"]') as HTMLTextAreaElement, 'Regards,\nGeorge\n{{clipboard}}'));
    const permission = container.querySelector('[aria-label="Allow clipboard reading"]') as HTMLInputElement;
    await act(async () => permission.click());
    await act(async () => setValue(container.querySelector('[aria-label="Voice Command test phrase"]') as HTMLTextAreaElement, 'mail signature'));
    await act(async () => button(container, 'Test').click());
    expect(mocks.preview).toHaveBeenCalledWith(expect.objectContaining({
      payload: { kind: 'snippet', trigger: 'mail signature', body: 'Regards,\nGeorge\n{{clipboard}}' },
      scope: { kind: 'app', bundleId: 'com.apple.mail' },
      voiceCommand: { commandType: 'snippet', allowClipboardRead: true },
    }), 'mail signature', false);
    expect(container.textContent).toContain('Yesterday:');
    await act(async () => button(container, 'Save command').click());
    expect(mocks.upsert).toHaveBeenCalledWith(expect.objectContaining({
      enabled: true,
      scope: { kind: 'app', bundleId: 'com.apple.mail' },
      voiceCommand: { commandType: 'snippet', allowClipboardRead: true },
    }));
  });

  it('duplicates into an independent validated draft without reading the clipboard', async () => {
    const sourceRow = Array.from(container.querySelectorAll('li')).find((row) => row.textContent?.includes('mail signature')) as HTMLLIElement;
    await act(async () => button(sourceRow, 'Duplicate').click());

    expect(container.querySelector('[role="dialog"]')?.textContent).toContain('Duplicate Voice Command');
    expect((container.querySelector('[aria-label="Voice Command type"]') as HTMLSelectElement).value).toBe('snippet');
    expect((container.querySelector('[aria-label="Voice Command scope"]') as HTMLSelectElement).value).toBe('app');
    expect((container.querySelector('[aria-label="Voice Command application"]') as HTMLSelectElement).value).toBe('com.apple.mail');
    expect((container.querySelector('[aria-label="Voice Command phrase"]') as HTMLInputElement).value).toBe('');
    expect((container.querySelector('[aria-label="Voice Command test phrase"]') as HTMLTextAreaElement).value).toBe('');
    expect((container.querySelector('[aria-label="Voice Command content"]') as HTMLTextAreaElement).value).toBe('Regards,\nGeorge\n{{clipboard}}');
    expect((container.querySelector('[aria-label="Allow clipboard reading"]') as HTMLInputElement).checked).toBe(true);
    expect((container.querySelector('input[type="checkbox"]:not([aria-label])') as HTMLInputElement).checked).toBe(false);
    expect(mocks.preview).not.toHaveBeenCalled();
    expect(mocks.upsert).not.toHaveBeenCalled();

    await act(async () => button(container, 'Save command').click());
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('Enter a spoken phrase.');
    expect(mocks.upsert).not.toHaveBeenCalled();

    const phrase = container.querySelector('[aria-label="Voice Command phrase"]') as HTMLInputElement;
    for (const character of 'alternate mail signature') {
      await act(async () => setValue(phrase, `${phrase.value}${character}`));
    }
    expect((container.querySelector('[aria-label="Voice Command test phrase"]') as HTMLTextAreaElement).value).toBe('alternate mail signature');
    await act(async () => button(container, 'Test').click());
    expect(mocks.preview).toHaveBeenCalledWith({
      payload: { kind: 'snippet', trigger: 'alternate mail signature', body: 'Regards,\nGeorge\n{{clipboard}}' },
      enabled: false,
      scope: { kind: 'app', bundleId: 'com.apple.mail' },
      voiceCommand: { commandType: 'snippet', allowClipboardRead: true },
    }, 'alternate mail signature', false);
    const previewPhrase = container.querySelector('[aria-label="Voice Command test phrase"]') as HTMLTextAreaElement;
    await act(async () => setValue(previewPhrase, 'custom preview phrase'));
    await act(async () => setValue(phrase, 'alternate mail signature two'));
    expect(previewPhrase.value).toBe('custom preview phrase');

    mocks.upsert.mockRejectedValueOnce(new Error('same-scope phrase conflict'));
    await act(async () => button(container, 'Save command').click());
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('same-scope phrase conflict');
    expect(container.textContent).toContain('mail signature');

    await act(async () => button(container, 'Save command').click());
    const savedDraft = mocks.upsert.mock.calls[mocks.upsert.mock.calls.length - 1]?.[0];
    expect(savedDraft).toEqual({
      payload: { kind: 'snippet', trigger: 'alternate mail signature two', body: 'Regards,\nGeorge\n{{clipboard}}' },
      enabled: false,
      scope: { kind: 'app', bundleId: 'com.apple.mail' },
      voiceCommand: { commandType: 'snippet', allowClipboardRead: true },
    });
    expect(savedDraft).not.toHaveProperty('id');
    expect(savedDraft).not.toHaveProperty('expectedRevision');
    expect(mocks.preview).toHaveBeenCalledTimes(1);
  });
});
