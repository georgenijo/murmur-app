import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
const mock = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mock.invoke }));
import { SourceRecovery } from './SourceRecovery';
it.each([false, true])('requires a selected backup and shows restore outcome on failure=%s', async failure => {
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command: string) => {
    if (command === 'get_dictation_settings_unsaved') return false;
    if (failure) throw new Error('synthetic failure');
    return undefined;
  });
  const host = document.createElement('div'); const root = createRoot(host);
  await act(async () => root.render(<SourceRecovery />));
  expect(host.textContent).toContain('No defaults will be published');
  expect(mock.invoke.mock.calls.map(([command]) => command)).toEqual(['get_dictation_settings_unsaved']);
  const input = host.querySelector('input')!;
  Object.defineProperty(input, 'files', { value: [{ text: async () => '{"synthetic":"backup"}' }] });
  await act(async () => { input.dispatchEvent(new Event('change', { bubbles: true })); });
  expect(mock.invoke).toHaveBeenCalledWith('restore_source_backup', { key: 'dictation-settings', blob: '{"synthetic":"backup"}' });
  expect(host.querySelector('[role="status"]')!.textContent).toContain(failure ? 'Backup could not be restored' : 'Backup restored');
  await act(async () => root.unmount());
});

it.each([false, true])('completion recovery requires explicit action and shows outcome on failure=%s', async failure => {
  mock.invoke.mockReset();
  mock.invoke.mockImplementation(async (command: string) => {
    if (command === 'get_dictation_settings_unsaved') return false;
    if (failure) throw new Error('synthetic failure');
    return undefined;
  });
  const host = document.createElement('div'); const root = createRoot(host);
  await act(async () => root.render(<SourceRecovery />));
  expect(mock.invoke.mock.calls.map(([command]) => command)).toEqual(['get_dictation_settings_unsaved']);
  await act(async () => { host.querySelector('button')!.click(); });
  expect(mock.invoke).toHaveBeenCalledWith('recover_completion_pending');
  expect(host.querySelector('[role="status"]')!.textContent).toContain(failure ? 'could not finish' : 'finished');
  await act(async () => root.unmount());
});
