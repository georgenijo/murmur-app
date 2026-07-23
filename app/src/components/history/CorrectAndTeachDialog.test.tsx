import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CorrectAndTeachDialog } from './CorrectAndTeachDialog';

const mocks = vi.hoisted(() => ({
  propose: vi.fn(),
  proposeSpecific: vi.fn(),
  confirm: vi.fn(),
  discard: vi.fn(async () => {}),
}));

vi.mock('../../lib/correctAndTeach', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../lib/correctAndTeach')>(),
  proposeLearnedCorrection: mocks.propose,
  proposeSpecificLearnedCorrection: mocks.proposeSpecific,
  confirmLearnedCorrection: mocks.confirm,
  discardLearnedCorrectionProposal: mocks.discard,
}));

function button(container: HTMLElement, label: string) {
  return Array.from(container.querySelectorAll('button')).find((candidate) => candidate.textContent?.trim() === label) as HTMLButtonElement;
}

function setValue(element: HTMLTextAreaElement | HTMLInputElement, value: string) {
  const prototype = element instanceof HTMLTextAreaElement
    ? HTMLTextAreaElement.prototype
    : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set;
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
    mocks.proposeSpecific.mockResolvedValue({
      kind: 'proposal', proposalId: 23, source: 'recording', replacement: 'recordingState',
      occurrenceCount: 1, originalText: 'use recording state', correctedText: 'use recordingState state',
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

  it('prefills the specific-term flow from the smallest safe automatic proposal', async () => {
    await act(async () => setValue(container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement, 'useRecordingState'));
    await act(async () => button(container, 'Review correction').click());
    await act(async () => button(container, 'Teach specific term').click());

    expect((container.querySelector('[aria-label="Exact heard term"]') as HTMLInputElement).value)
      .toBe('use recording state');
    expect((container.querySelector('[aria-label="Exact written replacement"]') as HTMLInputElement).value)
      .toBe('useRecordingState');
    expect(mocks.discard).toHaveBeenCalledWith(7);
    expect(mocks.confirm).not.toHaveBeenCalled();
  });

  it('supports keyboard text selection and requires review before remembering a specific rule', async () => {
    await act(async () => setValue(container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement, 'useRecordingState'));
    await act(async () => button(container, 'Review correction').click());
    await act(async () => button(container, 'Teach specific term').click());

    const heard = container.querySelector('[aria-label="Heard transcript for term selection"]') as HTMLTextAreaElement;
    heard.focus();
    heard.setSelectionRange(4, 13);
    await act(async () => heard.dispatchEvent(new KeyboardEvent('keyup', { key: 'ArrowRight', bubbles: true })));
    await act(async () => button(container, 'Use selected text').click());
    expect((container.querySelector('[aria-label="Exact heard term"]') as HTMLInputElement).value).toBe('recording');

    await act(async () => setValue(
      container.querySelector('[aria-label="Exact written replacement"]') as HTMLInputElement,
      'recordingState',
    ));
    await act(async () => button(container, 'Review specific term').click());

    expect(mocks.proposeSpecific).toHaveBeenCalledWith(
      'use recording state',
      'recording',
      'recordingState',
      undefined,
    );
    expect(container.textContent).toContain('Affects 1 exact occurrence');
    expect(container.textContent).toContain('use recordingState state');
    expect(mocks.confirm).not.toHaveBeenCalled();

    await act(async () => button(container, 'Remember correction').click());
    expect(mocks.confirm).toHaveBeenCalledWith(23, { kind: 'global' });
    expect(onSave).toHaveBeenCalledWith('useRecordingState');
  });

  it('shows specific validation failures without creating a confirmable proposal', async () => {
    mocks.propose.mockResolvedValueOnce({ kind: 'unsafe', reason: 'This edit is ambiguous.' });
    mocks.proposeSpecific.mockResolvedValueOnce({ kind: 'unsafe', reason: 'The heard term must match at least one whole term in this example.' });
    await act(async () => setValue(container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement, 'different'));
    await act(async () => button(container, 'Review correction').click());
    await act(async () => button(container, 'Teach specific term').click());
    await act(async () => setValue(container.querySelector('[aria-label="Exact heard term"]') as HTMLInputElement, 'missing'));
    await act(async () => setValue(container.querySelector('[aria-label="Exact written replacement"]') as HTMLInputElement, 'present'));
    await act(async () => button(container, 'Review specific term').click());

    expect(container.querySelector('[role="alert"]')?.textContent).toContain('whole term');
    expect(mocks.confirm).not.toHaveBeenCalled();
  });

  it('discards a specific proposal on Back and closes from the backdrop without persisting', async () => {
    await act(async () => setValue(container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement, 'useRecordingState'));
    await act(async () => button(container, 'Review correction').click());
    await act(async () => button(container, 'Teach specific term').click());
    await act(async () => button(container, 'Review specific term').click());

    await act(async () => button(container, 'Back').click());
    expect(mocks.discard).toHaveBeenCalledWith(23);
    expect(container.querySelector('[aria-label="Exact heard term"]')).not.toBeNull();

    const backdrop = container.firstElementChild as HTMLDivElement;
    await act(async () => backdrop.dispatchEvent(new MouseEvent('mousedown', { bubbles: true })));
    expect(onClose).toHaveBeenCalledOnce();
    expect(mocks.confirm).not.toHaveBeenCalled();
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

  it('keeps focus inside the labeled narrow-layout dialog and Escape never persists', async () => {
    const dialog = container.querySelector('[role="dialog"]') as HTMLDivElement;
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.getAttribute('aria-labelledby')).toBe('correct-and-teach-title');
    expect(dialog.className).toContain('max-h-[calc(100vh-1.5rem)]');
    expect(dialog.className).toContain('sm:max-h-[88vh]');

    await act(async () => setValue(container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement, 'useRecordingState'));
    await act(async () => button(container, 'Review correction').click());
    const last = button(container, 'Remember correction');
    last.focus();
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Tab', bubbles: true, cancelable: true,
    })));
    expect(document.activeElement).toBe(container.querySelector('[aria-label="Close Correct and Teach"]'));

    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'Escape', bubbles: true, cancelable: true,
    })));
    expect(onClose).toHaveBeenCalledOnce();
    expect(mocks.confirm).not.toHaveBeenCalled();
  });
});
