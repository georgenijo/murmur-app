import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CorrectionTeaching } from './CorrectionTeaching';

const api = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: api.invoke }));

describe('correction teaching', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    api.invoke.mockReset();
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('stores a rule only after a separate reviewed confirmation', async () => {
    api.invoke.mockImplementation(async (command: string) => command === 'propose_learned_correction'
      ? { kind: 'proposal', proposalId: 7, source: 'Tori', replacement: 'TAURI',
          occurrenceCount: 1, originalText: 'Use Tori.', correctedText: 'Use TAURI.',
          scopeOptions: [{ label: 'Everywhere', scope: { kind: 'global' } }] }
      : undefined);
    await act(async () => root.render(<CorrectionTeaching original="Use Tori." proposed="Use TAURI." />));
    expect(api.invoke).not.toHaveBeenCalled();
    await act(async () => container.querySelector('button')?.click());
    expect(container.textContent).toContain('Remember “Tori” → “TAURI”');
    expect(api.invoke.mock.calls.some(([command]) => command === 'confirm_learned_correction')).toBe(false);
    await act(async () => container.querySelector('button')?.click());
    expect(api.invoke).toHaveBeenCalledWith('confirm_learned_correction', { proposalId: 7, scope: { kind: 'global' } });
    expect(container.textContent).toContain('Remembered for future dictations.');
  });

  it('does not offer confirmation for an unsafe replacement', async () => {
    api.invoke.mockResolvedValue({ kind: 'unsafe', reason: 'The source phrase is ambiguous.' });
    await act(async () => root.render(<CorrectionTeaching original="Tori and Tori" proposed="TAURI and Tori" />));
    await act(async () => container.querySelector('button')?.click());
    expect(container.textContent).toContain('The source phrase is ambiguous.');
    expect(container.querySelector('button')).toBeNull();
    expect(api.invoke.mock.calls.some(([command]) => command === 'confirm_learned_correction')).toBe(false);
  });
});
