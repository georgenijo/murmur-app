import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const diagnostics = vi.hoisted(() => ({
  arm: vi.fn(async () => ({ state: 'armed' as const, expiresAtMs: Date.now() + 60_000 })),
  delete: vi.fn(async () => undefined),
  disarm: vi.fn(async () => ({ state: 'unarmed' as const })),
  get: vi.fn(async () => ({
    schemaVersion: 1 as const,
    captureId: 'capture-one',
    recordingId: 41,
    capturedAtMs: 10,
    expiresAtMs: 20,
    result: {
      kind: 'success' as const,
      rawText: { text: 'private raw', truncated: false },
      finalText: { text: 'private final', truncated: false },
      modelId: 'test-model',
      totalMs: 42,
    },
  })),
  list: vi.fn(async () => [{
    captureId: 'capture-one',
    recordingId: 41,
    capturedAtMs: 10,
    expiresAtMs: 20,
    outcome: 'success',
    hasContent: true,
  }]),
  status: vi.fn(async () => ({ state: 'unarmed' as const })),
  upload: vi.fn(async () => undefined),
}));

vi.mock('../../lib/dictationDiagnostics', () => ({
  armNextDictationCapture: diagnostics.arm,
  deleteDictationCapture: diagnostics.delete,
  disarmNextDictationCapture: diagnostics.disarm,
  getDictationCapture: diagnostics.get,
  getDictationCaptureStatus: diagnostics.status,
  listDictationCaptures: diagnostics.list,
  uploadDictationCapture: diagnostics.upload,
}));

import { DictationDiagnosticsView } from './DictationDiagnosticsView';

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const match = Array.from(container.querySelectorAll('button'))
    .find(candidate => candidate.textContent?.includes(label));
  if (!(match instanceof HTMLButtonElement)) throw new Error(`Missing button: ${label}`);
  return match;
}

describe('DictationDiagnosticsView consent flow', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    for (const mockFunction of Object.values(diagnostics)) mockFunction.mockClear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<DictationDiagnosticsView />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it('arms, disarms, reviews, uploads, and deletes through separate confirmations', async () => {
    await act(async () => button(container, 'Capture next dictation').click());
    expect(diagnostics.arm).toHaveBeenCalledOnce();
    expect(container.textContent).toContain('Armed · expires in');

    await act(async () => button(container, 'Disarm').click());
    expect(diagnostics.disarm).toHaveBeenCalledOnce();

    await act(async () => button(container, 'Review').click());
    expect(container.textContent).toContain('private raw');
    expect(container.textContent).toContain('private final');

    await act(async () => button(container, 'Upload reviewed capture').click());
    expect(diagnostics.upload).toHaveBeenCalledWith('capture-one');
    expect(window.confirm).toHaveBeenCalledTimes(2);

    await act(async () => button(container, 'Delete local copy').click());
    expect(diagnostics.delete).toHaveBeenCalledWith('capture-one');
    expect(window.confirm).toHaveBeenCalledTimes(3);
  });
});
