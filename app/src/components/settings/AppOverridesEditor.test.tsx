import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AppProfile } from '../../lib/settings';
import { AppOverridesEditor, overrideChoice, overrideValue } from './AppOverridesEditor';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));

const TERMINAL: AppProfile = {
  bundleId: 'com.apple.Terminal',
  label: 'Terminal',
  autoPasteOverride: null,
  cleanupOverride: null,
  smartFormattingOverride: null,
  cliFormattingOverride: null,
  writingStyle: null,
  ideContextEnabled: false,
  ideProjectRoots: [],
};

describe('AppOverridesEditor', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onChange = vi.fn();

  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue([
      { name: 'Terminal', bundleId: 'com.apple.Terminal' },
      { name: 'Safari', bundleId: 'com.apple.Safari' },
    ]);
    onChange.mockReset();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('maps explicit override choices without changing stored tri-state semantics', () => {
    expect([null, true, false].map(overrideChoice)).toEqual(['inherit', 'always', 'never']);
    expect(['inherit', 'always', 'never'].map((choice) => overrideValue(choice as 'inherit' | 'always' | 'never'))).toEqual([null, true, false]);
  });

  it('adds a running app from the memory-only picker', async () => {
    await act(async () => root.render(<AppOverridesEditor profiles={[]} onChange={onChange} />));
    await act(async () => { await Promise.resolve(); });
    const picker = container.querySelector('[aria-label="Running app"]') as HTMLSelectElement;
    await act(async () => {
      picker.value = 'com.apple.Terminal';
      picker.dispatchEvent(new Event('change', { bubbles: true }));
    });
    const add = Array.from(container.querySelectorAll('button')).find((button) => button.textContent === 'Add app') as HTMLButtonElement;
    await act(async () => add.click());
    expect(onChange).toHaveBeenCalledWith([TERMINAL]);
    expect(container.textContent).toContain('list stays in memory');
  });

  it('keeps manual bundle ID entry and reports duplicates visibly', async () => {
    await act(async () => root.render(<AppOverridesEditor profiles={[TERMINAL]} onChange={onChange} />));
    const bundle = container.querySelector('input[placeholder="com.apple.Terminal"]') as HTMLInputElement;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(bundle, 'COM.APPLE.TERMINAL');
      bundle.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const add = Array.from(container.querySelectorAll('button')).find((button) => button.textContent === 'Add') as HTMLButtonElement;
    await act(async () => add.click());
    expect(onChange).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('already exists');
  });

  it('writes explicit Always and Never values from labeled selects', async () => {
    await act(async () => root.render(<AppOverridesEditor profiles={[TERMINAL]} onChange={onChange} />));
    const select = container.querySelector('[aria-label="Transcript cleanup for Terminal"]') as HTMLSelectElement;
    await act(async () => {
      select.value = 'never';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    expect(onChange).toHaveBeenCalledWith([{ ...TERMINAL, cleanupOverride: false }]);
  });
});
