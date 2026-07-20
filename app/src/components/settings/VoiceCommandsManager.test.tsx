import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { KnowledgeEntry } from '../../lib/knowledge';
import { VoiceCommandsManager } from './VoiceCommandsManager';

const mocks = vi.hoisted(() => ({
  refresh: vi.fn(async () => {}),
  loadMore: vi.fn(async () => {}),
  upsert: vi.fn(async () => ({})),
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

vi.mock('../../lib/hooks/useKnowledge', () => ({
  useKnowledge: () => ({
    status: { availability: 'ready', schemaVersion: 3, recordCount: 1, storeRevision: 4, recoveryAtMs: null, message: null },
    entries: [ENTRY], total: 1, nextOffset: null, loading: false, error: null,
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
});
