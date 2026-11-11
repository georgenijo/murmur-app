import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { OnboardingFlow } from './OnboardingFlow';
import { ModelDownloader } from '../ModelDownloader';
import { useSettings } from '../../lib/hooks/useSettings';
import { DEFAULT_SETTINGS, type ModelOption } from '../../lib/settings';
import { isOnboardingComplete, markOnboardingComplete } from '../../lib/onboarding';
import { buildConfigureOptions, configure } from '../../lib/dictation';

const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(async (_event: string, _listener: (event: { payload: unknown }) => void) => () => {}),
  emit: vi.fn(async () => {}),
  progressListener: null as null | ((event: { payload: unknown }) => void),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke, isTauri: () => false }));
vi.mock('@tauri-apps/api/event', () => ({ listen: native.listen, emit: native.emit }));
vi.mock('@tauri-apps/plugin-autostart', () => ({
  isEnabled: vi.fn(async () => false), enable: vi.fn(), disable: vi.fn(),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

const COREML: ModelOption = 'parakeet-tdt-0.6b-v3-coreml';

describe('onboarding model state with native IPC boundaries', () => {
  let container: HTMLDivElement;
  let root: Root;
  let catalog: Array<{ modelName: ModelOption; installState: string }>;
  let settings: ReturnType<typeof useSettings>;
  let completed: ReturnType<typeof vi.fn<(...args: unknown[]) => void>>;

  const button = (label: string) => {
    const result = Array.from(container.querySelectorAll('button')).find((element) => label === 'Download' ? element.textContent?.trim().startsWith('Download ·') : element.textContent?.trim() === label);
    expect(result, `Button ${label}`).toBeDefined();
    return result!;
  };
  const click = async (label: string) => { await act(async () => button(label).click()); };
  const select = async (model: ModelOption) => {
    const row = container.querySelector<HTMLButtonElement>(`[id="download-model-${model}"]`)!;
    expect(row).not.toBeNull();
    await act(async () => row.click());
  };
  const expectSelected = (model: ModelOption) => {
    expect(container.querySelector(`[id="download-model-${model}"]`)?.getAttribute('aria-pressed')).toBe('true');
  };
  const mount = async (initialModel: ModelOption = COREML) => {
    localStorage.setItem('dictation-settings', JSON.stringify({ ...DEFAULT_SETTINGS, model: initialModel }));
    function Harness() {
      settings = useSettings();
      return <OnboardingFlow initialModel={initialModel} recordingMode="hold_down" triggerKey="shift_l" onComplete={async (model, recordingMode, doubleTapKey) => {
        await settings.configureOnboarding({ model, recordingMode, doubleTapKey });
        markOnboardingComplete();
        completed(model, recordingMode, doubleTapKey);
      }} />;
    }
    await act(async () => root.render(<Harness />));
    await click('Get started');
    await click('Continue');
    await click('Continue');
    await click('Skip for now');
    await click('Skip for now');
  };
  const done = async () => {
    await click('Continue'); // model -> hotkey
    await click('Double-tap');
    await act(async () => container.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
    await act(async () => Array.from(container.querySelectorAll<HTMLElement>('[role="option"]')).find(option => option.textContent?.trim() === 'F12')!.click());
    await click('Continue'); // hotkey -> summary
    await click('Start using Murmur');
  };

  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
    completed = vi.fn();
    native.progressListener = null;
    native.listen.mockImplementation(async (event: string, listener: (event: { payload: unknown }) => void) => {
      if (event === 'download-progress') native.progressListener = listener;
      return () => {};
    });
    catalog = [{ modelName: COREML, installState: 'installed' }, { modelName: 'base.en', installState: 'installed' }];
    native.invoke.mockImplementation(async (command: string, args?: { modelName?: ModelOption }) => {
      if (command === 'get_model_runtime_catalog') return catalog;
      if (command === 'check_microphone_permission_status') return 'granted';
      if (command === 'check_accessibility_permission') return true;
      if (command === 'get_system_audio_permission_status') return 'unknown';
      if (command === 'get_calendar_permission_status') return 'notDetermined';
      if (command === 'download_model') {
        catalog = [...catalog.filter((model) => model.modelName !== args?.modelName), { modelName: args!.modelName!, installState: 'installed' }];
      }
      return undefined;
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it('blocks catalog failures without Installed claims and retries (#945)', async () => {
    const implementation = native.invoke.getMockImplementation()!;
    native.invoke.mockImplementation((command, ...args) => command === 'get_model_runtime_catalog'
      ? Promise.reject(new Error('IPC unavailable')) : implementation(command, ...args));
    await mount();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('Could not check installed models');
    expect(container.textContent).not.toContain('Installed');
    expect(container.querySelector('[id^="download-model-"]')).toBeNull();
    expect(completed).not.toHaveBeenCalled();
    expect(isOnboardingComplete()).toBe(false);
    native.invoke.mockImplementation(implementation);
    await click('Retry');
    expectSelected(COREML);
    await done();
    expect(completed).toHaveBeenCalledWith(COREML, 'double_tap', 'f12');
  });

  it.each(['tiny.en', 'small.en', 'medium.en'] as const)('keeps installed %s selected even alongside curated models (#946)', async (model) => {
    catalog.push({ modelName: model, installState: 'installed' });
    await mount(model);
    expectSelected(model);
    await done();
    expect(completed).toHaveBeenCalledWith(model, 'double_tap', 'f12');
    expect(native.invoke.mock.calls.some(([command]) => command === 'download_model')).toBe(false);
  });

  it.each(['tiny.en', 'small.en', 'medium.en'] as const)('offers installed %s in the standalone gate (#946)', async (model) => {
    catalog = [{ modelName: model, installState: 'installed' }];
    await act(async () => root.render(<ModelDownloader initialModel={COREML} onComplete={completed} />));
    expectSelected(model);
    expect(container.querySelector('[id="download-model-base.en"]')).not.toBeNull();
    await click('Continue');
    expect(completed).toHaveBeenCalledWith(model);
    expect(native.invoke.mock.calls.some(([command]) => command === 'download_model')).toBe(false);
  });

  it('uses an installed non-curated model when the initial model is missing (#946)', async () => {
    catalog = [{ modelName: 'tiny.en', installState: 'installed' }];
    await mount();
    expectSelected('tiny.en');
    await done();
    expect(completed).toHaveBeenCalledWith('tiny.en', 'double_tap', 'f12');
  });

  it('keeps the standalone catalog failure retryable without offering unverified models (#945)', async () => {
    const implementation = native.invoke.getMockImplementation()!;
    native.invoke.mockImplementation((command, ...args) => command === 'get_model_runtime_catalog'
      ? Promise.reject(new Error('IPC unavailable')) : implementation(command, ...args));
    await act(async () => root.render(<ModelDownloader initialModel={COREML} onComplete={completed} />));
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('Could not check installed models');
    expect(container.querySelector('[id^="download-model-"]')).toBeNull();
    native.invoke.mockImplementation(implementation);
    await click('Retry');
    expectSelected(COREML);
    expect(completed).not.toHaveBeenCalled();
  });

  it.each([false, true])('preserves Base through Back (new download: %s) (#955)', async (download) => {
    if (download) catalog[1].installState = 'notInstalled';
    await mount();
    await select('base.en');
    await click(download ? 'Download' : 'Continue');
    await click('Back');
    expectSelected('base.en');
    await done();
    expect(completed).toHaveBeenCalledWith('base.en', 'double_tap', 'f12');
  });

  it('keeps download status and Cancel outside the scrolling model list', async () => {
    const pending = deferred<void>();
    const implementation = native.invoke.getMockImplementation()!;
    native.invoke.mockImplementation((command, ...args) => command === 'download_model'
      ? pending.promise : implementation(command, ...args));
    await mount();
    await select('large-v3-turbo');
    await click('Download');

    expect(container.querySelector('.onboarding-content')?.textContent).not.toContain('Cancel');
    const status = container.querySelector<HTMLElement>('.onboarding-model-status');
    expect(status).not.toBeNull();
    expect(container.querySelector('.onboarding-actions')?.textContent).toContain('Cancel');
    expect(status!.querySelector('[role="progressbar"]')).not.toBeNull();

    const requestId = native.invoke.mock.calls.find(([command]) => command === 'download_model')![1].requestId;
    await act(async () => native.progressListener?.({
      payload: {
        modelName: 'large-v3-turbo',
        requestId,
        attemptId: 1,
        received: 20,
        total: 100,
        phase: 'downloading',
      },
    }));
    expect(status!.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')).toBe('20');
    expect(status!.textContent).toContain('20%');
    await act(async () => pending.resolve());
  });

  it('cancels through the real footer and unlocks navigation only after native termination', async () => {
    const download = deferred<void>();
    const cancellation = deferred<void>();
    const implementation = native.invoke.getMockImplementation()!;
    native.invoke.mockImplementation((command, ...args) => command === 'download_model'
      ? download.promise : command === 'cancel_model_download' ? cancellation.promise : implementation(command, ...args));
    await mount();
    await select('large-v3-turbo');
    await click('Download');
    const request = native.invoke.mock.calls.find(([command]) => command === 'download_model')![1];
    const back = () => container.querySelector<HTMLButtonElement>('[aria-label="Go back to the previous setup step"]')!;
    expect(back().disabled).toBe(true);
    const cancel = container.querySelector<HTMLButtonElement>('.onboarding-actions button[data-variant="outline"]');
    expect(cancel?.textContent).toBe('Cancel');
    await act(async () => cancel!.click());
    expect(native.invoke).toHaveBeenCalledWith('cancel_model_download', request);
    expect(button('Cancelling…').disabled).toBe(true);
    expect(back().disabled).toBe(true);
    await act(async () => cancellation.resolve());
    expect(back().disabled).toBe(true); // The download must settle too.
    await act(async () => download.resolve());
    expect(back().disabled).toBe(false);
    expect(button('Download').disabled).toBe(false);
    expect([...container.querySelectorAll<HTMLButtonElement>('.model-option')].every((row) => !row.disabled)).toBe(true);
    expect(container.querySelector('[role="progressbar"]')).toBeNull();
    expect(completed).not.toHaveBeenCalled();
    await click('Back');
    expect(container.querySelector('h1')?.textContent).toBe('Calendar access');
  });

  it('waits for acknowledgment before persisting the combined choices or completion (#951)', async () => {
    const pending = deferred<unknown>();
    const implementation = native.invoke.getMockImplementation()!;
    native.invoke.mockImplementation((command, ...args) => command === 'configure_dictation' ? pending.promise : implementation(command, ...args));
    await mount();
    await select('base.en');
    await done();
    expect(button('Configuring model…').disabled).toBe(true);
    expect(button('Back').disabled).toBe(true);
    expect(isOnboardingComplete()).toBe(false);
    expect(settings.settings.model).toBe(COREML);
    expect(JSON.parse(localStorage.getItem('dictation-settings')!).model).toBe(COREML);
    expect(native.invoke.mock.calls.filter(([command]) => command === 'configure_dictation')).toHaveLength(1);
    await act(async () => pending.resolve({ type: 'configured' }));
    expect(isOnboardingComplete()).toBe(true);
    expect(settings.settings).toMatchObject({ model: 'base.en', recordingMode: 'double_tap', doubleTapKey: 'f12' });
    expect(JSON.parse(localStorage.getItem('dictation-settings')!)).toMatchObject({ model: 'base.en', recordingMode: 'double_tap', doubleTapKey: 'f12' });
  });

  it('keeps failures in the wizard with choices intact and supports Retry (#951)', async () => {
    const implementation = native.invoke.getMockImplementation()!;
    native.invoke.mockImplementation((command, ...args) => command === 'configure_dictation'
      ? Promise.reject(new Error('backend busy')) : implementation(command, ...args));
    await mount();
    await select('base.en');
    await done();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('selected transcription model');
    expect(container.textContent).not.toContain('vocabulary aliases');
    expect(isOnboardingComplete()).toBe(false);
    expect(settings.settings.model).toBe(COREML);
    native.invoke.mockImplementation(implementation);
    await click('Retry');
    expect(completed).toHaveBeenCalledWith('base.en', 'double_tap', 'f12');
    expect(isOnboardingComplete()).toBe(true);
  });

  it('publishes acknowledged choices before a queued startup configure reads them (#951)', async () => {
    const pending = deferred<unknown>();
    const implementation = native.invoke.getMockImplementation()!;
    native.invoke.mockImplementation((command, ...args) => command === 'configure_dictation' ? pending.promise : implementation(command, ...args));
    await mount();
    await select('base.en');
    await done();
    const startup = configure(() => buildConfigureOptions(settings.getConfirmedSettings()));
    await act(async () => { pending.resolve({ type: 'configured' }); await startup; });
    const writes = native.invoke.mock.calls.filter(([command]) => command === 'configure_dictation');
    expect(writes).toHaveLength(2);
    expect(writes[1][1].options.model).toBe('base.en');
    expect(completed).toHaveBeenCalledWith('base.en', 'double_tap', 'f12');
  });

  it('keeps the acknowledged model when overlay auto-paste changes during setup (#951)', async () => {
    const pending = deferred<unknown>();
    const implementation = native.invoke.getMockImplementation()!;
    native.invoke.mockImplementation((command, ...args) => command === 'configure_dictation' ? pending.promise : implementation(command, ...args));
    await mount();
    await select('base.en');
    await done();
    await act(async () => settings.applyExternalSettings({ autoPaste: true }));
    expect(isOnboardingComplete()).toBe(false);
    await act(async () => pending.resolve({ type: 'configured' }));
    const writes = native.invoke.mock.calls.filter(([command]) => command === 'configure_dictation');
    expect(writes).toHaveLength(2);
    expect(writes[1][1].options).toEqual({ autoPaste: true });
    expect(settings.getConfirmedSettings()).toMatchObject({ model: 'base.en', autoPaste: true });
    expect(settings.settings).toMatchObject({ model: 'base.en', autoPaste: true });
    expect(JSON.parse(localStorage.getItem('dictation-settings')!)).toMatchObject({ model: 'base.en', autoPaste: true });
    expect(isOnboardingComplete()).toBe(true);
    expect(completed).toHaveBeenCalledWith('base.en', 'double_tap', 'f12');
  });

  it('preserves rollback of an earlier failed edit before configuring setup choices (#951)', async () => {
    const pending = deferred<unknown>();
    const implementation = native.invoke.getMockImplementation()!;
    let configurations = 0;
    native.invoke.mockImplementation((command, ...args) => {
      if (command === 'configure_dictation' && ++configurations === 1) return pending.promise;
      return implementation(command, ...args);
    });
    await mount();
    await act(async () => settings.updateSettings({ language: 'fr' }));
    await select('base.en');
    await done();
    expect(isOnboardingComplete()).toBe(false);
    await act(async () => pending.reject(new Error('backend busy')));
    const writes = native.invoke.mock.calls.filter(([command]) => command === 'configure_dictation');
    expect(writes).toHaveLength(2);
    expect(writes[1][1].options).toMatchObject({ model: 'base.en', language: DEFAULT_SETTINGS.language });
    expect(settings.getConfirmedSettings()).toMatchObject({ model: 'base.en', language: DEFAULT_SETTINGS.language });
    expect(isOnboardingComplete()).toBe(true);
  });

  it('rechecks the chosen model at completion (#945)', async () => {
    await mount();
    catalog = [];
    await done();
    expect(isOnboardingComplete()).toBe(false);
    expect(completed).not.toHaveBeenCalled();
    expect(native.invoke.mock.calls.some(([command]) => command === 'configure_dictation')).toBe(false);
    expect(container.querySelector('[role="alert"]')).not.toBeNull();
  });

  it('times out configuration without committing a late acknowledgment (#951)', async () => {
    const pending = deferred<unknown>();
    const implementation = native.invoke.getMockImplementation()!;
    native.invoke.mockImplementation((command, ...args) => command === 'configure_dictation' ? pending.promise : implementation(command, ...args));
    await mount();
    await select('base.en');
    vi.useFakeTimers();
    await done();
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
    expect(isOnboardingComplete()).toBe(false);
    expect(button('Retry').disabled).toBe(false);
    await act(async () => pending.resolve({ type: 'configured' }));
    expect(isOnboardingComplete()).toBe(false);
    expect(settings.getConfirmedSettings().model).toBe(COREML);
    expect(settings.settings.model).toBe(COREML);
    expect(completed).not.toHaveBeenCalled();
    await click('Retry');
    expect(isOnboardingComplete()).toBe(true);
  });
});
