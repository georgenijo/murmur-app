import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ModelDownloadPanel } from '../../components/ModelDownloader';
import type { ModelOption } from '../settings';
import {
  useModelRuntimeCatalog,
  type ModelInstallState,
  type ModelRuntimeSnapshot,
} from '../modelRuntime';
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

function CatalogHarness({ enabled = true }: { enabled?: boolean }) {
  const { installStates, error, retry } = useInstalledModels(enabled);
  return (
    <div>
      <button type="button" onClick={retry}>Refresh</button>
      <span data-testid="base-en-state">{installStates?.['base.en'] ?? 'loading'}</span>
      {error && <span role="alert">Could not check installed models.</span>}
    </div>
  );
}

function RuntimeCatalogHarness() {
  const [revision, setRevision] = useState(0);
  const { models, byName, error, loaded } = useModelRuntimeCatalog(true, revision);
  return (
    <div>
      <button type="button" onClick={() => setRevision((value) => value + 1)}>Retry catalog</button>
      <span data-testid="runtime-loaded">{loaded ? 'loaded' : 'loading'}</span>
      <span data-testid="runtime-by-name">{byName.get('base.en')?.installState ?? 'missing'}</span>
      <span data-testid="runtime-models">
        {models.map((model) => `${model.modelName}:${model.generation}`).join(',')}
      </span>
      {error && <span role="alert">Could not read the runtime catalog.</span>}
    </div>
  );
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

  const baseEnState = () => container.querySelector('[data-testid="base-en-state"]')?.textContent;

  it('replaces an equal-generation snapshot when the catalog is refetched', async () => {
    let state: ModelInstallState = 'notInstalled';
    mocks.invoke.mockImplementation((command: string) => command === 'get_model_runtime_catalog'
      ? Promise.resolve([snapshot('base.en', state, 0)])
      : Promise.resolve(null));

    await act(async () => root.render(<CatalogHarness />));
    await settle();
    expect(baseEnState()).toBe('notInstalled');

    state = 'installed';
    await act(async () => button('Refresh').click());
    await settle();
    expect(baseEnState()).toBe('installed');
  });

  it('refreshes an equal-generation snapshot when the subscription re-enters', async () => {
    let state: ModelInstallState = 'notInstalled';
    mocks.invoke.mockImplementation((command: string) => command === 'get_model_runtime_catalog'
      ? Promise.resolve([snapshot('base.en', state, 0)])
      : Promise.resolve(null));

    await act(async () => root.render(<CatalogHarness enabled />));
    await settle();
    expect(baseEnState()).toBe('notInstalled');

    await act(async () => root.render(<CatalogHarness enabled={false} />));
    await settle();
    state = 'installed';
    await act(async () => root.render(<CatalogHarness enabled />));
    await settle();
    expect(baseEnState()).toBe('installed');
  });

  it('keeps a higher-generation event received before the catalog response', async () => {
    let resolveCatalog!: (catalog: ModelRuntimeSnapshot[]) => void;
    const catalog = new Promise<ModelRuntimeSnapshot[]>((resolve) => {
      resolveCatalog = resolve;
    });
    mocks.invoke.mockImplementation((command: string) => command === 'get_model_runtime_catalog'
      ? catalog
      : Promise.resolve(null));

    await act(async () => root.render(<CatalogHarness />));
    await settle();

    await emit(snapshot('base.en', 'installed', 5));
    expect(baseEnState()).toBe('loading');

    await act(async () => resolveCatalog([snapshot('base.en', 'notInstalled', 0)]));
    await settle();
    expect(baseEnState()).toBe('installed');
  });

  it('still ignores a stale lower-generation event after load', async () => {
    mocks.invoke.mockImplementation((command: string) => command === 'get_model_runtime_catalog'
      ? Promise.resolve([snapshot('base.en', 'installed', 5)])
      : Promise.resolve(null));

    await act(async () => root.render(<CatalogHarness />));
    await settle();
    expect(baseEnState()).toBe('installed');

    await emit(snapshot('base.en', 'notInstalled', 4));
    expect(baseEnState()).toBe('installed');
  });

  describe('useModelRuntimeCatalog after a failed catalog request', () => {
    const runtimeByName = () => container.querySelector('[data-testid="runtime-by-name"]')?.textContent;
    const runtimeLoaded = () => container.querySelector('[data-testid="runtime-loaded"]')?.textContent;
    const runtimeModels = () => container.querySelector('[data-testid="runtime-models"]')?.textContent;

    it('replays buffered events and applies later events without a baseline', async () => {
      let rejectCatalog!: (reason: Error) => void;
      const catalog = new Promise<ModelRuntimeSnapshot[]>((_, reject) => {
        rejectCatalog = reject;
      });
      mocks.invoke.mockImplementation((command: string) => command === 'get_model_runtime_catalog'
        ? catalog
        : Promise.resolve(null));

      await act(async () => root.render(<RuntimeCatalogHarness />));
      await settle();

      await emit(snapshot('base.en', 'installed', 2));
      expect(runtimeModels()).not.toContain('base.en');

      await act(async () => rejectCatalog(new Error('IPC unavailable')));
      await settle();
      expect(container.querySelector('[role="alert"]')).not.toBeNull();
      expect(runtimeLoaded()).toBe('loading');
      expect(runtimeByName()).toBe('installed');
      expect(runtimeModels()).toContain('base.en:2');

      await emit(snapshot('base.en', 'invalid', 3));
      expect(runtimeByName()).toBe('invalid');
      expect(runtimeModels()).toContain('base.en:3');
      expect(container.querySelector('[role="alert"]')).not.toBeNull();
    });

    it('starts a fresh baseline when a retry succeeds', async () => {
      let failure = true;
      mocks.invoke.mockImplementation((command: string) => {
        if (command === 'get_model_runtime_catalog') {
          return failure
            ? Promise.reject(new Error('IPC unavailable'))
            : Promise.resolve([snapshot('base.en', 'notInstalled', 0)]);
        }
        return Promise.resolve(null);
      });

      await act(async () => root.render(<RuntimeCatalogHarness />));
      await settle();
      expect(container.querySelector('[role="alert"]')).not.toBeNull();

      await emit(snapshot('base.en', 'installed', 9));
      expect(runtimeByName()).toBe('installed');

      failure = false;
      await act(async () => button('Retry catalog').click());
      await settle();
      expect(container.querySelector('[role="alert"]')).toBeNull();
      expect(runtimeLoaded()).toBe('loaded');
      expect(runtimeByName()).toBe('notInstalled');
      expect(runtimeModels()).toContain('base.en:0');
    });
  });
});
