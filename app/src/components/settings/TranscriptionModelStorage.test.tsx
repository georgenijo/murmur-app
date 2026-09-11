import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ModelRuntimeSnapshot } from '../../lib/modelRuntime';
import { TranscriptionModelStorage } from './TranscriptionModelStorage';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
let container: HTMLDivElement;
let root: Root;
const model = (modelName: string, extra: Partial<ModelRuntimeSnapshot> = {}): ModelRuntimeSnapshot => ({
  modelName, label: modelName, size: '75 MB', generation: 1, backend: 'whisper',
  accelerator: 'Metal GPU', supported: true, supportedPlatforms: ['macos'], unavailableReason: null,
  installState: 'installed', lifecycleState: 'unloaded', failurePresent: false,
  capabilities: { partialResults: false, initialPrompts: false, multilingual: false, translation: false,
    timestamps: false, confidence: false, punctuationControl: false }, ...extra,
});
const button = (name: string) => container.querySelector(`button[aria-label="${name}"]`) as HTMLButtonElement | null;
async function render(models = [model('base.en'), model('tiny.en')], busy = false, selectedModel = 'base.en') {
  await act(async () => root.render(<TranscriptionModelStorage models={models} selectedModel={selectedModel} busy={busy} />));
}
beforeEach(() => {
  container = document.createElement('div'); document.body.appendChild(container);
  root = createRoot(container); invoke.mockReset(); invoke.mockResolvedValue(undefined);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

it('requires two clicks, protects selected/active models, and reflects removal with Download', async () => {
  await render([model('base.en'), model('tiny.en'), model('small.en', { lifecycleState: 'ready' })]);
  expect(button('Remove base.en')).toBeNull(); expect(button('Remove small.en')).toBeNull();
  await act(async () => button('Remove tiny.en')!.click());
  expect(invoke).not.toHaveBeenCalled();
  await act(async () => button('Confirm remove tiny.en')!.click());
  expect(invoke).toHaveBeenCalledExactlyOnceWith('remove_model', { modelName: 'tiny.en' });
  await render([model('base.en'), model('tiny.en', { installState: 'notInstalled', generation: 2 })]);
  expect(button('Remove tiny.en')).toBeNull(); expect(container.textContent).toContain('Not installed');
  await act(async () => button('Download tiny.en')!.click());
  expect(invoke).toHaveBeenLastCalledWith('download_model', { modelName: 'tiny.en' });
});
it('clears confirmation on blur, selection changes, and busy transitions', async () => {
  await render();
  await act(async () => { button('Remove tiny.en')!.focus(); button('Remove tiny.en')!.click(); });
  await act(async () => button('Confirm remove tiny.en')!.blur());
  expect(button('Remove tiny.en')).not.toBeNull();
  await act(async () => button('Remove tiny.en')!.click());
  await render(undefined, true);
  expect(button('Remove tiny.en')!.disabled).toBe(true);
  await render(undefined, false, 'tiny.en');
  expect(button('Remove tiny.en')).toBeNull();
  expect(invoke).not.toHaveBeenCalled();
});
it('shows refusal without removing the installed row and prevents duplicate requests', async () => {
  let reject: (reason: string) => void = () => {};
  invoke.mockImplementation(() => new Promise((_, fail) => { reject = fail; }));
  await render();
  await act(async () => button('Remove tiny.en')!.click());
  await act(async () => button('Confirm remove tiny.en')!.click());
  expect(button('Remove tiny.en')!.disabled).toBe(true);
  await act(async () => button('Remove tiny.en')!.click());
  expect(invoke).toHaveBeenCalledTimes(1);
  await act(async () => reject('Wait for transcription to finish'));
  expect(container.querySelector('[role="alert"]')?.textContent).toContain('Wait for transcription');
  expect(button('Remove tiny.en')!.disabled).toBe(false);
});
it('does not offer removal or download during installation', async () => {
  await render([model('tiny.en', { installState: 'installing' })]);
  expect(container.querySelector('button')).toBeNull();
});
