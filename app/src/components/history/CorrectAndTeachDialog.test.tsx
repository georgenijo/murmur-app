import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CorrectAndTeachDialog } from './CorrectAndTeachDialog';

const mocks = vi.hoisted(() => ({
  propose: vi.fn(),
  confirm: vi.fn(),
  discard: vi.fn(async () => {}),
}));

vi.mock('../../lib/correctAndTeach', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../lib/correctAndTeach')>(),
  proposeLearnedCorrection: mocks.propose,
  confirmLearnedCorrection: mocks.confirm,
  discardLearnedCorrectionProposal: mocks.discard,
}));

function button(container: HTMLElement, label: string) {
  return Array.from(container.querySelectorAll('button')).find((candidate) => candidate.textContent?.trim() === label) as HTMLButtonElement;
}

function setValue(element: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  setter?.call(element, value);
  element.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('CorrectAndTeachDialog', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onClose = vi.fn();
  const onSave = vi.fn();

  beforeEach(async () => {
    vi.clearAllMocks();
    mocks.propose.mockResolvedValue({
      kind: 'proposal', proposalId: 7, source: 'use recording state', replacement: 'useRecordingState',
      occurrenceCount: 1, originalText: 'use recording state', correctedText: 'useRecordingState',
      scopeOptions: [{ scope: { kind: 'global' }, label: 'All apps' }],
    });
    mocks.confirm.mockResolvedValue({ id: 'learned', provenance: 'learned_correction' });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<CorrectAndTeachDialog entry={{
      id: 'history-1', text: 'use recording state', timestamp: 1, duration: 2, source: 'recording',
    }} onClose={onClose} onSaveCorrection={onSave} />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('does not persist until the exact reviewed rule is explicitly confirmed', async () => {
    await act(async () => setValue(container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement, 'useRecordingState'));
    await act(async () => button(container, 'Review correction').click());
    expect(mocks.propose).toHaveBeenCalledWith('use recording state', 'useRecordingState', undefined);
    expect(container.textContent).toContain('use recording state');
    expect(container.textContent).toContain('useRecordingState');
    expect(mocks.confirm).not.toHaveBeenCalled();

    await act(async () => button(container, 'Remember correction').click());
    expect(mocks.confirm).toHaveBeenCalledWith(7, { kind: 'global' });
    expect(onSave).toHaveBeenCalledWith('useRecordingState');
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('supports correction-only saves without storing a rule', async () => {
    mocks.propose.mockResolvedValueOnce({ kind: 'unsafe', reason: 'This edit changes more than one distinct span.' });
    await act(async () => setValue(container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement, 'alpha and omega'));
    await act(async () => button(container, 'Review correction').click());
    expect(container.textContent).toContain('No automatic rule suggested');
    await act(async () => button(container, 'Save correction only').click());
    expect(mocks.confirm).not.toHaveBeenCalled();
    expect(onSave).toHaveBeenCalledWith('alpha and omega');
  });

  it('discards a reviewed proposal when cancelled', async () => {
    await act(async () => setValue(container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement, 'useRecordingState'));
    await act(async () => button(container, 'Review correction').click());
    await act(async () => (container.querySelector('[aria-label="Close Correct and Teach"]') as HTMLButtonElement).click());
    expect(mocks.discard).toHaveBeenCalledWith(7);
    expect(mocks.confirm).not.toHaveBeenCalled();
  });

  it('discards a reviewed proposal when its parent unmounts', async () => {
    await act(async () => setValue(container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement, 'useRecordingState'));
    await act(async () => button(container, 'Review correction').click());
    await act(async () => root.render(<></>));
    expect(mocks.discard).toHaveBeenCalledWith(7);
    expect(mocks.confirm).not.toHaveBeenCalled();
  });

  it('discards a proposal that resolves after cancellation', async () => {
    let resolveProposal!: (value: Awaited<ReturnType<typeof mocks.propose>>) => void;
    mocks.propose.mockImplementationOnce(() => new Promise((resolve) => { resolveProposal = resolve; }));
    await act(async () => setValue(container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement, 'useRecordingState'));
    await act(async () => button(container, 'Review correction').click());
    await act(async () => button(container, 'Cancel').click());

    await act(async () => resolveProposal({
      kind: 'proposal', proposalId: 19, source: 'use recording state', replacement: 'useRecordingState',
      occurrenceCount: 1, originalText: 'use recording state', correctedText: 'useRecordingState',
      scopeOptions: [{ scope: { kind: 'global' }, label: 'All apps' }],
    }));

    expect(mocks.discard).toHaveBeenCalledWith(19);
    expect(mocks.confirm).not.toHaveBeenCalled();
  });
});
