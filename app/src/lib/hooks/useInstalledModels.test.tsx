import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ModelDownloadPanel } from '../../components/ModelDownloader';
import type { ModelOption } from '../settings';
import type { ModelInstallState, ModelRuntimeSnapshot } from '../modelRuntime';
import { useInstalledModels } from './useInstalledModels';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, listener: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(event, listener);
    return () => mocks.listeners.delete(event);
  }),
}));

function snapshot(
  modelName: ModelOption,
  installState: ModelInstallState,
  generation = 1,
): ModelRuntimeSnapshot {
  return {
    generation,
    modelName,
    label: modelName,
    size: '1 MB',
    backend: 'whisper',
    accelerator: 'CPU',
    capabilities: {
      partialResults: false,
      initialPrompts: false,
      multilingual: false,
      translation: true,
      timestamps: true,
      confidence: true,
      punctuationControl: false,
    },
    supportedPlatforms: ['macos'],
    supported: true,
    unavailableReason: null,
    installState,
    lifecycleState: 'unloaded',
    failurePresent: false,
  };
}

function Harness({ initialModel }: { initialModel: ModelOption }) {
  const { installStates, error, retry } = useInstalledModels();
  if (error) {
    return <div role="alert">Could not check installed models.<button type="button" onClick={retry}>Retry</button></div>;
  }
  if (installStates === null) return <p role="status">Checking installed models…</p>;
  return <ModelDownloadPanel initialModel={initialModel} installStates={installStates} onComplete={() => {}} />;
}

describe('useInstalledModels live install state', () => {
  let container: HTMLDivElement;
  let root: Root;

  const settle = async () => {
    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
  };
  const button = (label: string) => {
    const match = Array.from(container.querySelectorAll('button')).find(
      (candidate) => candidate.textContent?.trim() === label,
    );
    expect(match, `Expected button "${label}"`).toBeDefined();
    return match!;
  };
  const emit = async (payload: ModelRuntimeSnapshot) => {
    const listener = mocks.listeners.get('model-runtime-status-changed');
    expect(listener, 'runtime status listener').toBeDefined();
    await act(async () => listener!({ payload }));
    await settle();
  };

  beforeEach(() => {
    mocks.listeners.clear();
    mocks.invoke.mockReset();
    mocks.invoke.mockImplementation((command: string, args?: { modelName?: string }) => {
      if (command === 'get_model_runtime_catalog') {
        return Promise.resolve([
          snapshot('parakeet-tdt-0.6b-v3-coreml', 'installing'),
          snapshot('base.en', 'invalid'),
          snapshot('large-v3-turbo', 'installed'),
        ]);
      }
      if (command === 'get_model_hardware_guidance') return Promise.resolve(null);
      return Promise.resolve({ command, args });
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('renders live install states and flips a model to installed without remounting', async () => {
    await act(async () => root.render(<Harness initialModel="parakeet-tdt-0.6b-v3-coreml" />));
    await settle();

    expect(container.querySelector('[id="download-model-large-v3-turbo"]')?.textContent).toContain('Installed');
    expect(container.querySelector('[id="download-model-base.en"]')?.textContent).toContain('Needs repair');
    expect(container.querySelector('[id="download-model-parakeet-tdt-0.6b-v3-coreml"]')?.textContent).toContain('Installing…');

    const installingAction = button('Installing…');
    expect(installingAction.hasAttribute('disabled')).toBe(true);
    expect(Array.from(container.querySelectorAll('button')).some(
      (candidate) => candidate.textContent?.trim() === 'Download',
    )).toBe(false);

    await act(async () => (container.querySelector('[id="download-model-base.en"]') as HTMLButtonElement).click());
    expect(button('Repair download').hasAttribute('disabled')).toBe(false);

    await emit(snapshot('parakeet-tdt-0.6b-v3-coreml', 'installed', 2));
    await act(async () => (container.querySelector('[id="download-model-parakeet-tdt-0.6b-v3-coreml"]') as HTMLButtonElement).click());

    expect(container.querySelector('[id="download-model-parakeet-tdt-0.6b-v3-coreml"]')?.textContent).toContain('Installed');
    expect(button('Continue').hasAttribute('disabled')).toBe(false);
  });

  it('ignores a stale lower-generation transition', async () => {
    await act(async () => root.render(<Harness initialModel="parakeet-tdt-0.6b-v3-coreml" />));
    await settle();

    await emit(snapshot('parakeet-tdt-0.6b-v3-coreml', 'installed', 7));
    expect(container.querySelector('[id="download-model-parakeet-tdt-0.6b-v3-coreml"]')?.textContent).toContain('Installed');

    await emit(snapshot('parakeet-tdt-0.6b-v3-coreml', 'installing', 6));
    expect(container.querySelector('[id="download-model-parakeet-tdt-0.6b-v3-coreml"]')?.textContent).toContain('Installed');
    expect(button('Continue').hasAttribute('disabled')).toBe(false);
  });

  it('keeps catalog failures retryable and clears the error on retry', async () => {
    let failure = true;
    const implementation = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation((command: string, ...args: unknown[]) => command === 'get_model_runtime_catalog' && failure
      ? Promise.reject(new Error('IPC unavailable'))
      : implementation(command, ...args));

    await act(async () => root.render(<Harness initialModel="base.en" />));
    await settle();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('Could not check installed models');

    failure = false;
    await act(async () => button('Retry').click());
    await settle();
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container.querySelector('[id="download-model-base.en"]')?.textContent).toContain('Needs repair');
  });
});
