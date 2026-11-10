import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, type Settings } from '../../lib/settings';
import { SettingsSurfaceActiveContext } from './SettingsSurfaceContext';
import type { ModelHardwareGuidance } from '../../lib/modelHardwareGuidance';
import {
  SETTINGS_CATEGORIES,
  SettingsPanel,
  type SettingsPageRequest,
  autoPasteDeliveryDescription,
  effectiveAutoPaste,
  fileOutputDeliveryDescription,
  isMacOSSettingsPlatform,
  resolvePage,
} from './SettingsPanel';

vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '0.18.0') }));
const coreMocks = vi.hoisted(() => ({
  notchPillInstalled: false,
  notchPillDetectionError: false,
  invoke: vi.fn(),
}));
const diagnosticsWorkspaceMock = vi.hoisted(() => vi.fn());
const soundCueMock = vi.hoisted(() => vi.fn());
const eventMocks = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  listen: vi.fn(),
}));
const guidanceMocks = vi.hoisted(() => ({
  current: {
    schemaVersion: 1,
    chip: 'Apple M5 Pro',
    chipTier: 'currentAppleSilicon',
    physicalMemoryGib: 48,
    modelMemoryBudgetMib: 8192,
    recommendedModel: 'parakeet-tdt-0.6b-v3-coreml',
    warnedModels: [],
  } as ModelHardwareGuidance | null,
}));
vi.mock('@tauri-apps/api/core', () => ({
  invoke: coreMocks.invoke,
}));
vi.mock('@tauri-apps/api/event', () => ({
  emit: vi.fn(async () => {}),
  listen: eventMocks.listen,
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
vi.mock('../../lib/soundCues', () => ({ playSoundCue: soundCueMock }));
vi.mock('../../lib/modelRuntime', () => ({ useModelRuntimeCatalog: () => ({ models: [], byName: new Map(), error: null }) }));
vi.mock('../../lib/modelHardwareGuidance', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../lib/modelHardwareGuidance')>(),
  useModelHardwareGuidance: () => guidanceMocks.current,
}));
vi.mock('../../lib/hooks/useVocabScan', () => ({
  useVocabScan: () => ({ status: 'idle', walker: null, stats: null, scan: vi.fn(), cancel: vi.fn() }),
}));
vi.mock('./AppOverridesEditor', () => ({ AppOverridesEditor: () => <div>App overrides editor</div> }));
vi.mock('./AppearanceSettings', () => ({ AppearanceSettings: () => <div>Appearance settings</div> }));
vi.mock('./KnowledgeManager', () => ({ KnowledgeManager: () => <div>Knowledge manager</div> }));
vi.mock('./PerformanceLab', () => ({ PerformanceLab: () => <div>Performance lab</div> }));
vi.mock('../log-viewer/DiagnosticsWorkspace', () => ({
  DiagnosticsWorkspace: (props: unknown) => {
    diagnosticsWorkspaceMock(props);
    return <div>Diagnostics workspace</div>;
  },
}));
vi.mock('./VocabularyAliasesEditor', () => ({ VocabularyAliasesEditor: () => <div>Vocabulary editor</div> }));
vi.mock('./VoiceCommandsManager', () => ({ VoiceCommandsManager: ({ globallyEnabled, onOpenVoiceCommandSettings }: {
  globallyEnabled: boolean; onOpenVoiceCommandSettings?: () => void;
}) => <div>Voice commands editor{!globallyEnabled && <button type="button" onClick={onOpenVoiceCommandSettings}>
  Open Voice commands &amp; spoken formatting
</button>}</div> }));
vi.mock('./VocabScanStrip', () => ({ VocabScanStrip: () => <div>Vocabulary scan</div> }));

beforeEach(() => {
  localStorage.removeItem('murmur.settings-disclosure.v1');
  eventMocks.listeners.clear();
  eventMocks.listen.mockReset();
  eventMocks.listen.mockImplementation(async (name: string, handler: (event: { payload: unknown }) => void) => {
    eventMocks.listeners.set(name, handler);
    return () => eventMocks.listeners.delete(name);
  });
  diagnosticsWorkspaceMock.mockClear();
  coreMocks.notchPillInstalled = false;
  coreMocks.notchPillDetectionError = false;
  coreMocks.invoke.mockReset();
  guidanceMocks.current = {
    schemaVersion: 1,
    chip: 'Apple M5 Pro',
    chipTier: 'currentAppleSilicon',
    physicalMemoryGib: 48,
    modelMemoryBudgetMib: 8192,
    recommendedModel: 'parakeet-tdt-0.6b-v3-coreml',
    warnedModels: [],
  };
  coreMocks.invoke.mockImplementation(async (command: string) => {
    if (command === 'get_audio_input_inventory') return {
      schemaVersion: 2,
      revision: 1,
      status: 'available',
      devices: [],
      defaultInputId: null,
      lidState: 'unknown',
      errorCode: null,
    };
    if (command === 'get_microphone_preview_status') {
      return {
        previewId: null,
        state: 'idle',
        stillConnecting: false,
        errorKind: null,
        message: null,
      };
    }
    if (command === 'start_microphone_preview') {
      return {
        previewId: 1,
        state: 'active',
        stillConnecting: false,
        errorKind: null,
        message: null,
      };
    }
    if (command === 'stop_microphone_preview') {
      return {
        previewId: null,
        state: 'idle',
        stillConnecting: false,
        errorKind: null,
        message: null,
      };
    }
    if (command === 'cancel_microphone_preview') return false;
    if (command === 'is_notchpill_installed') {
      if (coreMocks.notchPillDetectionError) throw new Error('detector unavailable');
      return coreMocks.notchPillInstalled;
    }
    return undefined;
  });
});

describe('SettingsPanel information architecture', () => {
  let container: HTMLDivElement;
  let root: Root;
  const scrollTo = vi.fn();
  const scrollIntoView = vi.fn();
  const onUpdateSettings = vi.fn();

  function renderPanel(
    isOpen = true,
    settings: Settings = DEFAULT_SETTINGS,
    pageRequest: SettingsPageRequest | null = null,
    extras: Pick<Parameters<typeof SettingsPanel>[0], 'onPageOpened' | 'activeRef'> = {},
  ) {
    void isOpen;
    return root.render(
      <SettingsPanel
        settings={settings}
        onUpdateSettings={onUpdateSettings}
        initialized
        status="idle"
        onResetStats={vi.fn()}
        onRerunSetup={vi.fn()}
        accessibilityGranted
        onCheckForUpdate={vi.fn(async () => {})}
        onDownloadUpdate={vi.fn()}
        onRestartUpdate={vi.fn()}
        onOpenUpdate={vi.fn()}
        updateStatus={{ phase: 'idle' }}
        configureError={null}
        pageRequest={pageRequest}
        {...extras}
      />,
    );
  }

  beforeEach(async () => {
    scrollTo.mockReset();
    scrollIntoView.mockReset();
    onUpdateSettings.mockReset();
    Object.defineProperty(HTMLElement.prototype, 'scrollTo', { value: scrollTo, configurable: true });
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { value: scrollIntoView, configurable: true });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => renderPanel());
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('opens on General and falls back to it for unknown pages', () => {
    expect(SETTINGS_CATEGORIES.map((category) => category.label)).toEqual([
      'General', 'Recording', 'Customize', 'Delivery', 'Meetings', 'Text & vocabulary', 'AI & models', 'Modes', 'Appearance',
    ]);
    const nav = container.querySelector('nav[aria-label="Settings pages"]') as HTMLElement;
    expect(Array.from(nav.querySelectorAll('button')).map((button) => button.textContent)).toEqual(SETTINGS_CATEGORIES.map((category) => category.label));
    expect(nav.querySelector('[aria-current="page"]')?.textContent).toBe('General');
    expect(container.querySelector('h1')?.textContent).toBe('General');
    expect(resolvePage(undefined)).toBe('general');
    expect(resolvePage('not-a-page')).toBe('general');
  });

  it('returns from every customization destination and restores focus to its row', async () => {
    const customize = Array.from(container.querySelectorAll<HTMLButtonElement>('nav[aria-label="Settings pages"] button'))
      .find((button) => button.textContent === 'Customize')!;
    await act(async () => customize.click());
    expect(Array.from(container.querySelectorAll('[aria-label="Customization destinations"] button')).map((button) => button.textContent)).toEqual([
      expect.stringContaining('Text & vocabulary'),
      expect.stringContaining('Voice commands'),
      expect.stringContaining('App overrides'),
      expect.stringContaining('Modes'),
    ]);
    const hubButton = (label: string) => Array.from(container.querySelectorAll<HTMLButtonElement>('[aria-label="Customization destinations"] button'))
      .find((button) => button.textContent?.includes(label)) as HTMLButtonElement;

    for (const [label, heading] of [
      ['Text & vocabulary', 'Text & vocabulary'],
      ['Voice commands', 'Voice commands'],
      ['App overrides', 'Delivery'],
      ['Modes', 'Modes'],
    ] as const) {
      const row = hubButton(label);
      await act(async () => row.click());
      expect((container.querySelector('[role="dialog"] h2') ?? container.querySelector('h1'))?.textContent).toBe(heading);
      const back = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
        (button) => button.textContent?.includes('Back to Customize'),
      ) as HTMLButtonElement;
      expect(back).toBeDefined();
      await act(async () => back.click());
      expect(container.querySelector('h1')?.textContent).toBe('Customize Murmur');
      expect(document.activeElement).toBe(hubButton(label));
    }
  });

  it('presents Customize as independent named destinations rather than numbered steps', async () => {
    await act(async () => renderPanel(true, DEFAULT_SETTINGS, { page: 'customize', token: 1 }));
    const list = container.querySelector('[aria-label="Customization destinations"]')!;
    expect(list.tagName).toBe('UL');
    expect(list.children).toHaveLength(4);
    expect(Array.from(list.children).every(element => element.tagName === 'LI')).toBe(true);
    expect(list.textContent).not.toMatch(/0[1-5]/);
  });

  it('closes the commands sheet and focuses the actual disabled switch without enabling it', async () => {
    await act(async () => renderPanel(true, { ...DEFAULT_SETTINGS, voiceCommandsEnabled: false }, { page: 'customize', token: 1 }));
    const commandsRow = () => Array.from(container.querySelectorAll<HTMLButtonElement>('[aria-label="Customization destinations"] button'))
      .find(button => button.textContent?.includes('Voice commands'))!;
    await act(async () => commandsRow().click());
    const link = Array.from(container.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'))
      .find(button => button.textContent === 'Open Voice commands & spoken formatting')!;
    expect(link).toBeDefined();
    await act(async () => link.click());
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(container.querySelector('h1')?.textContent).toBe('Text & vocabulary');
    expect(container.querySelector('.settings-sidebar')?.closest('[inert]')).toBeNull();
    const row = container.querySelector('[data-setting-target="voice-commands"]')!;
    const toggle = row.querySelector('[role="switch"]')!;
    expect(toggle.getAttribute('aria-label')).toBe('Voice commands & spoken formatting');
    expect(toggle.getAttribute('aria-checked')).toBe('false');
    expect(row.textContent).toContain('Voice commands & spoken formatting');
    expect(document.activeElement).toBe(toggle);
    expect(row.classList.contains('settings-target-flash')).toBe(true);
    expect(onUpdateSettings).not.toHaveBeenCalled();
    const back = container.querySelector<HTMLButtonElement>('.settings-back-btn')!;
    await act(async () => back.click());
    expect(container.querySelector('h1')?.textContent).toBe('Customize Murmur');
    expect(document.activeElement).toBe(commandsRow());
  });

  it.each(['none', 'pointer', 'keyboard'] as const)('waits for the editor link destination, preserves it across updates, and cancels pending focus on %s interaction', async (interaction) => {
    const frames = new Map<number, FrameRequestCallback>();
    let frameId = 0;
    let rowVisible = false;
    let switchVisible = false;
    const computedStyle = window.getComputedStyle.bind(window);
    const styleSpy = vi.spyOn(window, 'getComputedStyle').mockImplementation((element, pseudo) => {
      const visibility = element.matches('[data-setting-target="voice-commands"]') ? rowVisible
        : element.matches('[role="switch"][aria-label="Voice commands & spoken formatting"]') ? switchVisible
        : null;
      return visibility === null ? computedStyle(element, pseudo)
        : { visibility: visibility ? 'visible' : 'hidden' } as CSSStyleDeclaration;
    });
    const requestFrame = vi.spyOn(window, 'requestAnimationFrame').mockImplementation(callback => {
      frames.set(++frameId, callback);
      return frameId;
    });
    const cancelFrame = vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(id => { frames.delete(id); });
    const nextFrame = async () => act(async () => {
      const pending = Array.from(frames.values());
      frames.clear();
      pending.forEach(callback => callback(performance.now()));
    });
    const settings = { ...DEFAULT_SETTINGS, voiceCommandsEnabled: false };
    const request: SettingsPageRequest = { page: 'text', editorTab: 'commands', token: 1 };
    try {
      await act(async () => renderPanel(true, settings, request));
      const link = Array.from(container.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'))
        .find(button => button.textContent === 'Open Voice commands & spoken formatting')!;
      await act(async () => link.click());
      const row = container.querySelector<HTMLElement>('[data-setting-target="voice-commands"]')!;
      const toggle = row.querySelector<HTMLElement>('[role="switch"]')!;
      const focus = vi.spyOn(toggle, 'focus');
      await nextFrame();
      await nextFrame();
      expect(row.classList.contains('settings-target-flash')).toBe(false);
      expect(focus).not.toHaveBeenCalled();
      rowVisible = true;
      await nextFrame();
      expect(row.classList.contains('settings-target-flash')).toBe(true);
      expect(focus).not.toHaveBeenCalled();
      await act(async () => renderPanel(true, { ...settings, soundCuesEnabled: !settings.soundCuesEnabled }, request));
      const elsewhere = container.querySelector<HTMLInputElement>('input[type="search"]')!;
      if (interaction !== 'none') {
        await act(async () => {
          elsewhere.dispatchEvent(interaction === 'pointer'
            ? new Event('pointerdown', { bubbles: true })
            : new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
          elsewhere.focus();
        });
      }
      switchVisible = true;
      await nextFrame();
      if (interaction === 'none') {
        expect(document.activeElement).toBe(toggle);
        expect(focus).toHaveBeenCalledTimes(1);
        elsewhere.focus();
        await nextFrame();
        expect(document.activeElement).toBe(elsewhere);
        expect(focus).toHaveBeenCalledTimes(1);
      } else {
        expect(document.activeElement).toBe(elsewhere);
        expect(focus).not.toHaveBeenCalled();
      }
      expect(scrollIntoView.mock.instances).toContain(row);
      expect(onUpdateSettings).not.toHaveBeenCalled();
      focus.mockRestore();
    } finally {
      styleSpy.mockRestore();
      requestFrame.mockRestore();
      cancelFrame.mockRestore();
    }
  });

  it('reveals Voice commands from external navigation and search without moving focus to its switch', async () => {
    await act(async () => renderPanel(true, DEFAULT_SETTINGS, { page: 'text', target: 'voice-commands', token: 1 }));
    const toggle = container.querySelector<HTMLElement>('[data-setting-target="voice-commands"] [role="switch"]')!;
    expect(document.activeElement).not.toBe(toggle);
    expect(toggle.closest('[data-setting-target]')?.classList.contains('settings-target-flash')).toBe(true);
    const search = container.querySelector<HTMLInputElement>('input[type="search"]')!;
    search.focus();
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(search, 'Voice commands');
      search.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const result = Array.from(container.querySelectorAll<HTMLButtonElement>('button'))
      .find(button => button.textContent?.includes('Enable custom commands and spoken formatting tokens'))!;
    expect(result).toBeDefined();
    await act(async () => result.click());
    const revealedRow = container.querySelector<HTMLElement>('[data-setting-target="voice-commands"]')!;
    expect(document.activeElement).not.toBe(revealedRow.querySelector('[role="switch"]'));
    expect(revealedRow.classList.contains('settings-target-flash')).toBe(true);
    expect(onUpdateSettings).not.toHaveBeenCalled();
  });

  it('shows Modes and browser site rules on their own page', async () => {
    const modes = Array.from(container.querySelectorAll<HTMLButtonElement>('nav[aria-label="Settings pages"] button'))
      .find((button) => button.textContent === 'Modes') as HTMLButtonElement;

    await act(async () => modes.click());

    expect(container.querySelector('h1')?.textContent).toBe('Modes');
    const browserSites = container.querySelector('[data-setting-target="browser-sites"]');
    expect(browserSites?.querySelector('.settings-row-title')?.textContent).toBe('Browser sites');
    expect(browserSites?.closest('details')).toBeNull();
  });

  it.each([1, 2])('credits a retained page when Settings becomes visible with request %i', async (token) => {
    const activeRef = { current: false };
    const onPageOpened = vi.fn();
    const renderVisiblePanel = (visible: boolean, requestToken: number) => root.render(
      <SettingsSurfaceActiveContext.Provider value={visible}>
        <SettingsPanel
          settings={DEFAULT_SETTINGS}
          onUpdateSettings={onUpdateSettings}
          initialized
          status="idle"
          onResetStats={vi.fn()}
          onRerunSetup={vi.fn()}
          accessibilityGranted
          onCheckForUpdate={vi.fn(async () => {})}
          onDownloadUpdate={vi.fn()}
          onRestartUpdate={vi.fn()}
          onOpenUpdate={vi.fn()}
          updateStatus={{ phase: 'idle' }}
          configureError={null}
          pageRequest={{ page: 'shortcuts', token: requestToken }}
          onPageOpened={onPageOpened}
          activeRef={activeRef}
        />
      </SettingsSurfaceActiveContext.Provider>,
    );
    await act(async () => renderVisiblePanel(false, 1));
    expect(onPageOpened).not.toHaveBeenCalled();

    activeRef.current = true;
    await act(async () => renderVisiblePanel(true, token));
    expect(container.querySelector('h1')?.textContent).toBe('Keyboard shortcuts');
    expect(onPageOpened).toHaveBeenCalledExactlyOnceWith('shortcuts');
  });

  it('reports rendered General and shortcuts pages', async () => {
    const onPageOpened = vi.fn();
    await act(async () => renderPanel(true, DEFAULT_SETTINGS, null, {
      onPageOpened,
    }));
    expect(onPageOpened).toHaveBeenCalledWith('general');

    const shortcuts = Array.from(container.querySelectorAll<HTMLButtonElement>('button'))
      .find((button) => button.textContent?.includes('Keyboard shortcuts')) as HTMLButtonElement;
    await act(async () => shortcuts.click());
    expect(onPageOpened).toHaveBeenCalledWith('shortcuts');
  });

  it('routes an empty Mode binding through app creation and returns to binding', async () => {
    const customMode = {
      id: 'mode.focus', name: 'Focus', builtIn: false, enabled: true,
      writingStyle: null, cleanupEnabled: null, smartFormattingEnabled: null,
      cliFormattingEnabled: null, vocabularyPolicy: 'inherit' as const,
      contextPolicy: 'none' as const, modelId: null, language: null, autoPaste: null,
    };
    await act(async () => renderPanel(true, { ...DEFAULT_SETTINGS, modes: [customMode] }));
    const modes = Array.from(container.querySelectorAll<HTMLButtonElement>('nav[aria-label="Settings pages"] button'))
      .find((button) => button.textContent === 'Modes') as HTMLButtonElement;
    await act(async () => modes.click());
    const focusMode = Array.from(container.querySelectorAll<HTMLButtonElement>('button'))
      .find((button) => button.textContent?.includes('Focus')) as HTMLButtonElement;
    await act(async () => focusMode.click());
    const addApp = Array.from(container.querySelectorAll<HTMLButtonElement>('button'))
      .find((button) => button.textContent === 'Add an app') as HTMLButtonElement;
    await act(async () => addApp.click());
    expect(container.querySelector('h1')?.textContent).toBe('Modes');
    expect(container.querySelector('[role="dialog"] h2')?.textContent).toBe('App overrides');

    await act(async () => renderPanel(true, {
      ...DEFAULT_SETTINGS,
      modes: [customMode],
      appProfiles: [{
        bundleId: 'com.example.Editor',
        label: 'Editor',
        autoPasteOverride: null,
        cleanupOverride: null,
        smartFormattingOverride: null,
        cliFormattingOverride: null,
        writingStyle: null,
        ideContextEnabled: false,
        ideProjectRoots: [],
      }],
    }));
    expect(container.querySelector('h1')?.textContent).toBe('Modes');
    expect((container.querySelector('[aria-label="Mode name"]') as HTMLInputElement).value).toBe('Focus');
    expect(container.textContent).toContain('Editor');
  });

  it.each([false, true])('resets changed Auto-paste and respects file output=%s', async (saveTranscript) => {
    await act(async () => renderPanel(true, { ...DEFAULT_SETTINGS, autoPaste: false }, { page: 'delivery', token: 1 }));
    expect(container.querySelector('[data-setting-target="auto-paste"] [aria-label="Reset to default"]')).toBeNull();
    await act(async () => renderPanel(true, { ...DEFAULT_SETTINGS, autoPaste: true, saveTranscript }, { page: 'delivery', token: 1 }));
    const row = container.querySelector('[data-setting-target="auto-paste"]')!;
    const reset = row.querySelector<HTMLButtonElement>('[aria-label="Reset to default"]');
    if (saveTranscript) {
      const toggle = row.querySelector<HTMLElement>('[role="switch"]')!;
      expect(toggle.getAttribute('aria-disabled')).toBe('true');
      await act(async () => toggle.click());
      expect(!reset || reset.disabled).toBe(true);
      expect(onUpdateSettings).not.toHaveBeenCalled();
    } else {
      expect(reset).not.toBeNull();
      expect(reset!.disabled).toBe(false);
      await act(async () => reset!.click());
      expect(onUpdateSettings).toHaveBeenCalledExactlyOnceWith({ autoPaste: false });
    }
  });

  it.each(['keyboard', 'assistive'])('persists and labels Paste Delay changes from %s input', async (input) => {
    let persistedSettings = { ...DEFAULT_SETTINGS, autoPaste: true };
    onUpdateSettings.mockImplementation((patch: Partial<Settings>) => {
      persistedSettings = { ...persistedSettings, ...patch };
      renderPanel(true, persistedSettings, { page: 'delivery', token: 1 });
    });
    await act(async () => renderPanel(true, persistedSettings, { page: 'delivery', token: 1 }));
    const slider = container.querySelector<HTMLInputElement>('input[type="range"]')!;
    expect(slider).not.toBeNull();
    await act(async () => {
      // Use the native setter so React receives a genuine input change.
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(slider, '80');
      slider.dispatchEvent(new Event('input', { bubbles: true }));
      if (input === 'keyboard') slider.dispatchEvent(new KeyboardEvent('keyup', { key: 'ArrowRight', bubbles: true }));
    });
    expect(onUpdateSettings).toHaveBeenCalledWith({ autoPasteDelayMs: 80 });
    expect(slider.labels?.[0]?.textContent).toBe('Paste delay');
    await act(async () => renderPanel(true, persistedSettings, { page: 'general', token: 2 }));
    await act(async () => renderPanel(true, persistedSettings, { page: 'delivery', token: 3 }));
    expect(container.querySelector<HTMLInputElement>('input[type="range"]')!.value).toBe('80');
  });

  it('coalesces all pointer-drag input steps into one final Paste Delay commit', async () => {
    await act(async () => renderPanel(true, { ...DEFAULT_SETTINGS, autoPaste: true }, { page: 'delivery', token: 1 }));
    const slider = container.querySelector<HTMLInputElement>('input[type="range"]')!;
    await act(async () => slider.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 7, button: 0, isPrimary: true, bubbles: true })));
    for (let value = 10; value <= 500; value += 10) {
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(slider, String(value));
        slider.dispatchEvent(new Event('input', { bubbles: true }));
      });
    }
    expect(slider.value).toBe('500');
    expect(slider.getAttribute('aria-valuetext')).toBe('500 milliseconds');
    expect(onUpdateSettings).not.toHaveBeenCalled();
    await act(async () => slider.dispatchEvent(new PointerEvent('pointerup', { pointerId: 9, button: 0, bubbles: true })));
    expect(onUpdateSettings).not.toHaveBeenCalled();
    await act(async () => slider.dispatchEvent(new PointerEvent('pointerup', { pointerId: 7, button: 0, bubbles: true })));
    expect(onUpdateSettings).toHaveBeenCalledExactlyOnceWith({ autoPasteDelayMs: 500 });
    await act(async () => slider.dispatchEvent(new Event('input', { bubbles: true })));
    expect(onUpdateSettings).toHaveBeenCalledTimes(1);
  });

  it.each(['blur', 'pointercancel', 'lostpointercapture', 'window-blur', 'outside-release', 'unmount'])('flushes a Paste Delay draft once on %s', async (ending) => {
    await act(async () => renderPanel(true, { ...DEFAULT_SETTINGS, autoPaste: true }, { page: 'delivery', token: 1 }));
    const slider = container.querySelector<HTMLInputElement>('input[type="range"]')!;
    await act(async () => slider.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 7, button: 0, isPrimary: true, bubbles: true })));
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(slider, '120');
      slider.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(onUpdateSettings).not.toHaveBeenCalled();
    await act(async () => {
      if (ending === 'unmount') renderPanel(true, { ...DEFAULT_SETTINGS, autoPaste: true }, { page: 'general', token: 2 });
      else if (ending === 'blur') slider.dispatchEvent(new FocusEvent('focusout', { bubbles: true }));
      else if (ending === 'window-blur') window.dispatchEvent(new Event('blur'));
      else if (ending === 'outside-release') window.dispatchEvent(new PointerEvent('pointerup', { pointerId: 7, button: 0 }));
      else slider.dispatchEvent(new PointerEvent(ending, { pointerId: 7, bubbles: true }));
    });
    expect(onUpdateSettings).toHaveBeenCalledExactlyOnceWith({ autoPasteDelayMs: 120 });
    await act(async () => slider.dispatchEvent(new PointerEvent('pointerup', { pointerId: 7, button: 0, bubbles: true })));
    expect(onUpdateSettings).toHaveBeenCalledTimes(1);
  });

  it('commits keyboard changes to voice-detection sensitivity', async () => {
    const recording = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Recording',
    ) as HTMLButtonElement;
    await act(async () => recording.click());
    const slider = container.querySelector('[data-setting-target="voice-detection"] input[type="range"]') as HTMLInputElement;
    await act(async () => {
      slider.value = '75';
      slider.dispatchEvent(new Event('input', { bubbles: true }));
      slider.dispatchEvent(new KeyboardEvent('keyup', { key: 'ArrowRight', bubbles: true }));
    });

    expect(onUpdateSettings).toHaveBeenCalledWith({ vadSensitivity: 75 });
  });

  it('does not configure unchanged sensitivity on tab, pointer release, or blur', async () => {
    await act(async () => renderPanel(true, DEFAULT_SETTINGS, { page: 'recording', token: 1 }));
    const slider = container.querySelector<HTMLInputElement>('[data-setting-target="voice-detection"] input[type="range"]')!;
    await act(async () => {
      slider.focus();
      slider.dispatchEvent(new KeyboardEvent('keyup', { key: 'Tab', bubbles: true }));
      slider.dispatchEvent(new Event('pointerup', { bubbles: true }));
      slider.blur();
    });
    expect(onUpdateSettings).not.toHaveBeenCalled();
  });

  it.each(['pointerup', 'keyup', 'focusout'])('commits actual sensitivity changes via %s and allows retry after rollback', async (ending) => {
    const request = { page: 'recording', token: 1 };
    const initial = { ...DEFAULT_SETTINGS, vadSensitivity: 50 };
    await act(async () => renderPanel(true, initial, request));
    const slider = container.querySelector<HTMLInputElement>('[data-setting-target="voice-detection"] input[type="range"]')!;
    const change = async () => act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(slider, '75');
      slider.dispatchEvent(new Event('input', { bubbles: true }));
      slider.dispatchEvent(new Event(ending, { bubbles: true }));
    });
    await change();
    expect(onUpdateSettings).toHaveBeenCalledExactlyOnceWith({ vadSensitivity: 75 });
    await act(async () => renderPanel(true, { ...initial, vadSensitivity: 75 }, request));
    await act(async () => {
      slider.dispatchEvent(new KeyboardEvent('keyup', { key: 'Tab', bubbles: true }));
      slider.dispatchEvent(new FocusEvent('focusout', { bubbles: true }));
    });
    expect(onUpdateSettings).toHaveBeenCalledTimes(1);
    await act(async () => renderPanel(true, initial, request));
    expect(slider.value).toBe('50');
    await change();
    expect(onUpdateSettings).toHaveBeenCalledTimes(2);
  });

  it.each([0, 75])('labels Sensitivity with understandable value text at %s', async (vadSensitivity) => {
    await act(async () => renderPanel(true, { ...DEFAULT_SETTINGS, vadSensitivity }, { page: 'recording', token: 1 }));
    const slider = container.querySelector<HTMLInputElement>('[data-setting-target="voice-detection"] input[type="range"]')!;
    expect(slider.labels?.[0]?.textContent).toBe('Sensitivity');
    expect(slider.getAttribute('aria-valuetext')).toBe(vadSensitivity === 0 ? 'Off' : '75 percent');
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(slider, '85');
      slider.dispatchEvent(new Event('input', { bubbles: true }));
      slider.dispatchEvent(new FocusEvent('focusout', { bubbles: true }));
    });
    expect(slider.getAttribute('aria-valuetext')).toBe('85 percent');
    expect(onUpdateSettings).toHaveBeenCalledWith({ vadSensitivity: 85 });
  });

  it('shows a flat keyboard-operable trigger picker', async () => {
    await act(async () => renderPanel(true, { ...DEFAULT_SETTINGS, doubleTapKey: 'f12' }));
    const recording = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Recording',
    ) as HTMLButtonElement;
    await act(async () => recording.click());

    const picker = container.querySelector<HTMLButtonElement>('[aria-label="Dictation trigger key"]');
    expect(picker?.textContent).toContain('F12');
    await act(async () => picker?.click());
    const listbox = container.querySelector('[role="listbox"]')!;
    expect(listbox.querySelector('[role="group"]')).toBeNull();
    expect(listbox.querySelectorAll('[role="option"]')).toHaveLength(23);

    await act(async () => {
      picker?.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }));
    });
    await act(async () => {
      picker?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(onUpdateSettings).toHaveBeenCalledWith({ doubleTapKey: 'f20' });
    expect(container.textContent).toContain('May need Fn on Apple keyboards');
    expect(container.textContent).toContain('keeps its normal action');
  });

  it('updates and previews recording sound cues', async () => {
    const recording = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Recording',
    ) as HTMLButtonElement;
    await act(async () => recording.click());
    await act(async () => (container.querySelector('.settings-advanced-toggle') as HTMLButtonElement).click());
    const soundSwitch = container.querySelector('[role="switch"][aria-label="Sound cues"]') as HTMLElement;
    await act(async () => soundSwitch.click());
    expect(onUpdateSettings).toHaveBeenCalledWith({ soundCuesEnabled: false });
    const volumeLabel = Array.from(container.querySelectorAll('label')).find(
      (label) => label.textContent?.includes('Volume'),
    ) as HTMLLabelElement;
    const slider = volumeLabel.querySelector('input[type="range"]') as HTMLInputElement;
    const valueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      valueSetter.call(slider, '70');
      slider.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(onUpdateSettings).toHaveBeenCalledWith({ soundCueVolume: 70 });
    const success = Array.from(container.querySelectorAll('button')).find((button) => button.textContent === 'success') as HTMLButtonElement;
    await act(async () => success.click());
    expect(soundCueMock).toHaveBeenCalledWith('success', 45);
    const meetingSwitch = container.querySelector('[role="switch"][aria-label="Meeting cues"]') as HTMLElement;
    await act(async () => meetingSwitch.click());
    expect(onUpdateSettings).toHaveBeenCalledWith({ meetingSoundCuesEnabled: true });
  });

  it('renders a setting toggle as an accessible switch that reflects checked state and reports clicks', async () => {
    const recording = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Recording',
    ) as HTMLButtonElement;
    await act(async () => recording.click());
    await act(async () => (container.querySelector('.settings-advanced-toggle') as HTMLButtonElement).click());
    const soundSwitch = container.querySelector('[role="switch"][aria-label="Sound cues"]') as HTMLElement;
    expect(soundSwitch).toBeTruthy();
    expect(soundSwitch.getAttribute('aria-checked')).toBe('true');
    await act(async () => soundSwitch.click());
    expect(onUpdateSettings).toHaveBeenCalledWith({ soundCuesEnabled: false });
  });

  it('hides the NotchPill setting when the companion app is absent', () => {
    expect(container.textContent).not.toContain('Mirror captions to NotchPill');
  });

  it('does not re-enumerate microphones when the window receives focus', async () => {
    const before = coreMocks.invoke.mock.calls.filter(([command]) => command === 'get_audio_input_inventory').length;
    expect(before).toBeGreaterThan(0);
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      await Promise.resolve();
    });
    const after = coreMocks.invoke.mock.calls.filter(([command]) => command === 'get_audio_input_inventory').length;
    expect(after).toBe(before);
  });

  it('announces inventory failures without presenting stale choices', async () => {
    const recording = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Recording',
    ) as HTMLButtonElement;
    await act(async () => recording.click());
    await act(async () => {
      eventMocks.listeners.get('audio-input-inventory-changed')?.({ payload: {
        schemaVersion: 2,
        revision: 2,
        status: 'unavailable',
        devices: [],
        defaultInputId: null,
        lidState: 'unknown',
        errorCode: 'enumerationFailed',
      } });
    });
    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain('microphone list is temporarily unavailable');
    expect((container.querySelector('[aria-label="Microphone input"]') as HTMLButtonElement).disabled).toBe(true);
  });

  it('shows the NotchPill setting when the companion app is installed', async () => {
    const delivery = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Delivery',
    ) as HTMLButtonElement;
    await act(async () => delivery.click());
    coreMocks.notchPillInstalled = true;
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      await Promise.resolve();
    });

    expect(container.textContent).toContain('Mirror captions to NotchPill');
  });

  it('hides the NotchPill setting when detection fails', async () => {
    const delivery = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Delivery',
    ) as HTMLButtonElement;
    await act(async () => delivery.click());
    coreMocks.notchPillInstalled = true;
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      await Promise.resolve();
    });
    expect(container.textContent).toContain('Mirror captions to NotchPill');

    coreMocks.notchPillDetectionError = true;
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      await Promise.resolve();
    });

    expect(container.textContent).not.toContain('Mirror captions to NotchPill');
  });

  it('opens focused Recording, Meetings, Text, and AI destinations', async () => {
    for (const [page, expected] of [
      ['Recording', 'Voice detection'],
      ['Meetings', 'Keep meeting audio'],
      ['Text & vocabulary', 'Vocabulary'],
      ['AI & models', 'Speech-to-text'],
    ] as const) {
      const button = Array.from(container.querySelectorAll('nav button')).find((item) => item.textContent === page) as HTMLButtonElement;
      await act(async () => button.click());
      expect(container.textContent).toContain(expected);
      expect(button.getAttribute('aria-current')).toBe('page');
    }
    expect(scrollTo).toHaveBeenCalledWith({ top: 0 });
  });

  it('keeps the scroll position when Delivery Advanced opens and closes', async () => {
    const delivery = Array.from(container.querySelectorAll<HTMLButtonElement>('nav button'))
      .find(button => button.textContent === 'Delivery')!;
    await act(async () => delivery.click());
    const advanced = container.querySelector<HTMLButtonElement>('.settings-advanced-toggle')!;
    if (advanced.getAttribute('aria-expanded') === 'true') {
      await act(async () => advanced.click());
    }
    scrollTo.mockClear();
    scrollIntoView.mockClear();

    await act(async () => advanced.click());
    expect(advanced.getAttribute('aria-expanded')).toBe('true');
    expect(scrollTo).not.toHaveBeenCalled();
    expect(scrollIntoView).not.toHaveBeenCalled();

    await act(async () => advanced.click());
    expect(advanced.getAttribute('aria-expanded')).toBe('false');
    expect(scrollTo).not.toHaveBeenCalled();
    expect(scrollIntoView).not.toHaveBeenCalled();

    const general = Array.from(container.querySelectorAll<HTMLButtonElement>('nav button'))
      .find(button => button.textContent === 'General')!;
    await act(async () => general.click());
    expect(scrollTo).toHaveBeenCalledWith({ top: 0 });
  });

  it('discloses Calendar reads and browser limits before enabling meeting suggestions', async () => {
    const meetings = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Meetings',
    ) as HTMLButtonElement;
    await act(async () => meetings.click());

    const toggle = container.querySelector<HTMLButtonElement>(
      '[role="switch"][aria-label="Suggest Notetaker for Calendar meetings"]',
    );
    expect(toggle?.getAttribute('aria-checked')).toBe('false');
    expect(container.textContent).toContain('Reads your Calendar');
    expect(container.textContent).toContain('Zoom, Teams, Slack, Webex, FaceTime');
    expect(container.textContent).toContain('never reads URLs or window titles');

    await act(async () => toggle?.click());
    expect(onUpdateSettings).toHaveBeenCalledWith({ meetingSuggestionsEnabled: true });
  });

  it('accepts a direct Keyboard Shortcuts page request for command-palette navigation', async () => {
    await act(async () => root.render(
      <SettingsPanel
        settings={DEFAULT_SETTINGS}
        onUpdateSettings={onUpdateSettings}
        initialized
        status="idle"
        onResetStats={vi.fn()}
        onRerunSetup={vi.fn()}
        accessibilityGranted
        onCheckForUpdate={vi.fn(async () => {})}
        onDownloadUpdate={vi.fn()}
        onRestartUpdate={vi.fn()}
        onOpenUpdate={vi.fn()}
        updateStatus={{ phase: 'idle' }}
        configureError={null}
        pageRequest={{ page: 'shortcuts', token: 1 }}
      />,
    ));

    expect(resolvePage('shortcuts')).toBe('shortcuts');
    expect(container.querySelector('h1')?.textContent).toBe('Keyboard shortcuts');
    expect(container.querySelector('nav [aria-current="page"]')?.textContent).toBe('General');
  });

  it('honors initial and repeated target requests, then resets on normal navigation', async () => {
    await act(async () => root.unmount());
    root = createRoot(container);
    scrollTo.mockClear();
    scrollIntoView.mockClear();

    // Keep deferred recentering frames from racing these immediate-landing checks.
    vi.useFakeTimers();
    try {
      await act(async () => renderPanel(true, DEFAULT_SETTINGS, {
        page: 'recording',
        target: 'trigger-key',
        token: 1,
      }));
      const target = container.querySelector('[data-setting-target="trigger-key"]') as HTMLElement;
      expect(scrollIntoView).toHaveBeenCalledTimes(1);
      expect(scrollTo).not.toHaveBeenCalled();
      expect(target.classList.contains('settings-target-flash')).toBe(true);

      await act(async () => renderPanel(true, DEFAULT_SETTINGS, {
        page: 'recording',
        target: 'trigger-key',
        token: 2,
      }));
      expect(scrollIntoView).toHaveBeenCalledTimes(2);
      expect(target.classList.contains('settings-target-flash')).toBe(true);

      await act(async () => renderPanel(true, DEFAULT_SETTINGS, {
        page: 'general',
        token: 3,
      }));
      expect(container.querySelector('h1')?.textContent).toBe('General');
      expect(scrollTo).toHaveBeenCalledWith({ top: 0 });
      expect(target.classList.contains('settings-target-flash')).toBe(false);

      await act(async () => vi.advanceTimersByTime(2000));
      expect(scrollIntoView).toHaveBeenCalledTimes(2);
      expect(target.classList.contains('settings-target-flash')).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it('shows hardware guidance and keeps a warned speech model selectable', async () => {
    guidanceMocks.current = {
      schemaVersion: 1,
      chip: 'Apple M1',
      chipTier: 'legacyAppleSilicon',
      physicalMemoryGib: 8,
      modelMemoryBudgetMib: 1536,
      recommendedModel: 'parakeet-tdt-0.6b-v3-coreml',
      warnedModels: ['parakeet-tdt-0.6b-v2-fp16', 'medium.en', 'large-v3-turbo'],
    };
    await act(async () => renderPanel(true, { ...DEFAULT_SETTINGS, model: 'large-v3-turbo' }));
    const aiModels = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'AI & models',
    ) as HTMLButtonElement;
    await act(async () => aiModels.click());
    const speechModel = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.includes('Speech-to-text') && button.textContent?.includes('Configure'),
    ) as HTMLButtonElement;
    await act(async () => speechModel.click());

    expect(container.textContent).toContain('recommended for Apple M1 with 8 GB RAM');
    expect(container.textContent).toContain('You can still select and use it');
    const picker = container.querySelector('[role="combobox"]') as HTMLButtonElement;
    await act(async () => picker.click());
    const medium = Array.from(container.querySelectorAll('[role="option"]')).find(
      (option) => option.textContent?.includes('Whisper Medium'),
    ) as HTMLElement;
    expect(medium.textContent).toContain('Higher memory');
    await act(async () => medium.click());
    expect(onUpdateSettings).toHaveBeenCalledWith({ model: 'medium.en' });
  });

  it('opens editors as a sheet over Text settings with explicit close navigation', async () => {
    const settingsPages = container.querySelector('nav[aria-label="Settings pages"]') as HTMLElement;
    const textTab = Array.from(settingsPages.querySelectorAll('button')).find(
      (button) => button.textContent === 'Text & vocabulary',
    ) as HTMLButtonElement;
    await act(async () => textTab.click());

    const aliases = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.trim().startsWith('Aliases'),
    ) as HTMLButtonElement;
    await act(async () => aliases.click());

    expect(settingsPages.isConnected).toBe(true);
    expect(settingsPages.querySelector('[aria-current="page"]')?.textContent).toBe('Text & vocabulary');
    expect(container.querySelector('nav[aria-label="Settings editors"]')).toBeNull();
    expect(container.querySelector('[role="dialog"] h2')?.textContent).toBe('Aliases');

    const back = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.includes('Close editor'),
    ) as HTMLButtonElement;
    expect(back).toBeDefined();
    await act(async () => back.click());

    expect(container.textContent).toContain('Text & vocabulary');
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(settingsPages.querySelector('[aria-current="page"]')?.textContent).toBe('Text & vocabulary');
  });

  it('returns from an editor with Escape', async () => {
    const settingsPages = container.querySelector('nav[aria-label="Settings pages"]') as HTMLElement;
    const textTab = Array.from(settingsPages.querySelectorAll('button')).find(
      (button) => button.textContent === 'Text & vocabulary',
    ) as HTMLButtonElement;
    await act(async () => textTab.click());
    const vocabulary = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.trim().startsWith('Vocabulary'),
    ) as HTMLButtonElement;
    await act(async () => vocabulary.click());

    expect(container.querySelector('[role="dialog"] h2')?.textContent).toBe('Vocabulary');
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })));
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(settingsPages.querySelector('[aria-current="page"]')?.textContent).toBe('Text & vocabulary');
  });

  it('opens diagnostics as a dedicated tool destination', async () => {
    const button = Array.from(container.querySelectorAll('button')).find((item) => item.textContent === 'Diagnostics') as HTMLButtonElement;
    await act(async () => button.click());
    expect(container.textContent).toContain('Diagnostics workspace');
    expect(diagnosticsWorkspaceMock).toHaveBeenCalledWith(expect.objectContaining({
      storeHealthEnabled: true,
    }));
    expect(button.getAttribute('aria-current')).toBe('page');
  });

  it('searches across tabs and routes a result to its owning tab', async () => {
    const input = container.querySelector('input[type="search"]') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(input, 'appearance');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(container.textContent).toContain('1 result');
    const result = Array.from(container.querySelectorAll('button')).find((button) => button.textContent?.includes('Appearance')) as HTMLButtonElement;
    await act(async () => result.click());
    expect(container.querySelector('nav [aria-current="page"]')?.textContent).toBe('Appearance');
    expect(container.textContent).toContain('Appearance settings');
  });

  it('keeps the search name separate from its clear action and returns focus after clearing', async () => {
    const input = container.querySelector<HTMLInputElement>('input[type="search"]')!;
    expect(input.labels?.[0]?.textContent).toBe('Search all settings');
    expect(input.placeholder).toBe('Search');
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, 'overlay');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const clear = container.querySelector<HTMLButtonElement>('[aria-label="Clear settings search"]')!;
    expect(clear.closest('label')).toBeNull();
    expect(input.labels?.[0]?.textContent).toBe('Search all settings');
    clear.focus();
    await act(async () => clear.click());
    expect(input.value).toBe('');
    expect(document.activeElement).toBe(input);
    expect(container.querySelector('[aria-label="Clear settings search"]')).toBeNull();
    expect(container.querySelector('h1')?.textContent).toBe('General');
  });

  it('finds calibration and reopens General advanced settings for repeated searches', async () => {
    // Count immediate landings, not deferred animation frames racing the test.
    vi.useFakeTimers();
    try {
      const input = container.querySelector('input[type="search"]') as HTMLInputElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
      const searchCalibration = async () => {
        await act(async () => {
          setter.call(input, 'Calibrate');
          input.dispatchEvent(new Event('input', { bubbles: true }));
        });
        const result = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
          (button) => button.textContent?.includes('Calibrate overlay position'),
        ) as HTMLButtonElement;
        expect(result).toBeDefined();
        await act(async () => result.click());
      };

      scrollIntoView.mockClear();
      await searchCalibration();

      const calibration = container.querySelector('[data-setting-target="overlay-calibration"]') as HTMLElement;
      const advanced = calibration.closest('.settings-advanced')?.querySelector('button') as HTMLButtonElement;
      expect(container.querySelector('nav [aria-current="page"]')?.textContent).toBe('General');
      expect(advanced.getAttribute('aria-expanded')).toBe('true');
      expect(scrollIntoView).toHaveBeenCalledWith({ block: 'center' });
      expect(calibration.classList.contains('settings-target-flash')).toBe(true);

      // Searching unmounts General; selecting the same result must reveal the
      // newly mounted, initially collapsed disclosure again.
      await searchCalibration();

      expect((container.querySelector('[data-setting-target="overlay-calibration"]') as HTMLElement)
        .closest('.settings-advanced')?.querySelector('button')?.getAttribute('aria-expanded')).toBe('true');
      expect(scrollIntoView).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it.each([
    ['overlay', 'Calibrate overlay position', 'overlay-calibration', 'General'],
    ['overlay position', 'Calibrate overlay position', 'overlay-calibration', 'General'],
    ['paste delay', 'Paste delay', 'paste-delay', 'Delivery'],
    ['sound cues', 'Sound cues', 'sound-cues', 'Recording'],
  ])('searches for %s, opens its disclosure and preserves the target landing', async (query, title, targetId, heading) => {
    await act(async () => renderPanel(true, { ...DEFAULT_SETTINGS, autoPaste: true }));
    vi.useFakeTimers();
    try {
      const input = container.querySelector<HTMLInputElement>('input[type="search"]')!;
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, query);
        input.dispatchEvent(new Event('input', { bubbles: true }));
      });
      const result = Array.from(container.querySelectorAll<HTMLButtonElement>('[aria-label="Settings search results"] button'))
        .find(button => button.textContent?.startsWith(title))!;
      expect(result).toBeDefined();
      scrollTo.mockClear();
      await act(async () => result.click());
      const target = container.querySelector<HTMLElement>(`[data-setting-target="${targetId}"]`)!;
      expect(container.querySelector('h1')?.textContent).toBe(heading);
      expect(target).not.toBeNull();
      expect(target.closest('[aria-hidden="true"]')).toBeNull();
      expect(target.classList.contains('settings-target-flash')).toBe(true);
      expect(scrollIntoView.mock.instances).toContain(target);
      await act(async () => vi.advanceTimersByTime(2000));
      expect(scrollTo).not.toHaveBeenCalled();
      expect(onUpdateSettings).not.toHaveBeenCalled();
    } finally { vi.useRealTimers(); }
  });

  it('keeps the App overrides destination centered after its highlight expires', async () => {
    await act(async () => renderPanel(true, DEFAULT_SETTINGS, { page: 'customize', token: 1 }));
    const row = Array.from(container.querySelectorAll<HTMLButtonElement>('[aria-label="Customization destinations"] button'))
      .find(button => button.textContent?.includes('App overrides'))!;
    vi.useFakeTimers();
    try {
      scrollTo.mockClear();
      await act(async () => row.click());
      const target = container.querySelector('[data-setting-target="app-overrides"]')!;
      expect(target.closest('.settings-advanced')?.querySelector('button')?.getAttribute('aria-expanded')).toBe('true');
      expect(scrollIntoView.mock.instances).toContain(target);
      await act(async () => vi.advanceTimersByTime(2000));
      expect(scrollTo).not.toHaveBeenCalled();
      expect(container.querySelector('h1')?.textContent).toBe('Delivery');
    } finally { vi.useRealTimers(); }
  });

  it('preserves the parent scroll while entering a sheet and restores its page on close', async () => {
    const input = container.querySelector('input[type="search"]') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(input, 'vocabulary aliases');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const result = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
      (button) => button.textContent?.includes('Vocabulary & aliases'),
    ) as HTMLButtonElement;
    await act(async () => result.click());
    expect(scrollIntoView).toHaveBeenCalledWith({ block: 'center' });

    scrollTo.mockClear();
    const aliases = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
      (button) => button.textContent?.trim().startsWith('Aliases'),
    ) as HTMLButtonElement;
    await act(async () => aliases.click());
    expect(container.querySelector('[role="dialog"] h2')?.textContent).toBe('Aliases');
    expect(scrollTo).not.toHaveBeenCalled();

    scrollTo.mockClear();
    const back = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
      (button) => button.textContent?.includes('Close editor'),
    ) as HTMLButtonElement;
    await act(async () => back.click());
    expect(container.querySelector('h1')?.textContent).toBe('Text & vocabulary');
    expect(scrollTo).toHaveBeenCalledWith({ top: 0 });
  });

  it('opens diagnostics when the cross-tab result explicitly targets them', async () => {
    const input = container.querySelector('input[type="search"]') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(input, 'diagnostics');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });

    const result = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.includes('Diagnostics'),
    ) as HTMLButtonElement;
    expect(result.textContent).toContain('Diagnostics');
    await act(async () => result.click());

    const selectedTool = Array.from(container.querySelectorAll('button')).find(
      (button) => button.getAttribute('aria-current') === 'page' && button.textContent === 'Diagnostics',
    );
    expect(selectedTool).toBeTruthy();
    expect(container.textContent).toContain('Diagnostics workspace');
  });
  it('offers configurable and disabled Paste last choices in Delivery settings', async () => {
    const delivery = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Delivery',
    ) as HTMLButtonElement;
    await act(async () => delivery.click());
    const picker = container.querySelector('button[aria-label="Paste last shortcut"]') as HTMLButtonElement;
    expect(picker.textContent).toContain('Disabled');
    await act(async () => picker.click());
    expect(Array.from(container.querySelectorAll('[role="option"]')).map((option) => option.textContent)).toEqual([
      expect.stringContaining('Disabled'),
      expect.stringContaining('⌘⇧V'),
      expect.stringContaining('⌘⌥V'),
      expect.stringContaining('⌘⌃V'),
    ]);
    const option = Array.from(container.querySelectorAll('[role="option"]')).find(
      (item) => item.textContent?.includes('⌘⌥V'),
    ) as HTMLElement;
    await act(async () => option.click());
    expect(coreMocks.invoke).toHaveBeenCalledWith('set_paste_last_shortcut', {
      shortcut: 'command_option_v',
    });
    expect(onUpdateSettings).toHaveBeenCalledWith({ pasteLastShortcut: 'command_option_v' });
    await act(async () => root.render(
      <SettingsPanel
        settings={{ ...DEFAULT_SETTINGS, pasteLastShortcut: 'command_option_v' }}
        onUpdateSettings={onUpdateSettings}
        initialized
        status="idle"
        onResetStats={vi.fn()}
        onRerunSetup={vi.fn()}
        accessibilityGranted
        onCheckForUpdate={vi.fn(async () => {})}
        onDownloadUpdate={vi.fn()}
        onRestartUpdate={vi.fn()}
        onOpenUpdate={vi.fn()}
        updateStatus={{ phase: 'idle' }}
        configureError={null}
      />,
    ));
    expect(container.textContent).toContain('Press ⌘⌥V to paste your latest dictation again');
  });

  it('keeps the last working Paste last value when native registration fails', async () => {
    const current = { ...DEFAULT_SETTINGS, pasteLastShortcut: 'command_shift_v' as const };
    await act(async () => root.render(
      <SettingsPanel
        settings={current}
        onUpdateSettings={onUpdateSettings}
        initialized
        status="idle"
        onResetStats={vi.fn()}
        onRerunSetup={vi.fn()}
        accessibilityGranted
        onCheckForUpdate={vi.fn(async () => {})}
        onDownloadUpdate={vi.fn()}
        onRestartUpdate={vi.fn()}
        onOpenUpdate={vi.fn()}
        updateStatus={{ phase: 'idle' }}
        configureError={null}
      />,
    ));
    const delivery = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Delivery',
    ) as HTMLButtonElement;
    await act(async () => delivery.click());
    coreMocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'set_paste_last_shortcut') throw new Error('Shortcut registration failed');
      return undefined;
    });
    const picker = container.querySelector('button[aria-label="Paste last shortcut"]') as HTMLButtonElement;
    expect(picker.textContent).toContain('⌘⇧V');
    await act(async () => picker.click());
    const option = Array.from(container.querySelectorAll('[role="option"]')).find(
      (item) => item.textContent?.includes('⌘⌃V'),
    ) as HTMLElement;
    await act(async () => option.click());
    expect(onUpdateSettings).not.toHaveBeenCalledWith({ pasteLastShortcut: 'command_control_v' });
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('Shortcut registration failed');
    expect((container.querySelector('button[aria-label="Paste last shortcut"]') as HTMLButtonElement).textContent)
      .toContain('⌘⇧V');
  });

  it('marks the Auto-Paste switch data-disabled and ignores clicks while file output is on', async () => {
    // Base UI's Switch.Root renders a non-native <span role="switch">, which
    // never gets the native `disabled` attribute — only `data-disabled`. A
    // disabled-looking switch that still responds to clicks would silently
    // flip a setting the UI claims is unavailable.
    await act(async () => root.render(
      <SettingsPanel
        settings={{ ...DEFAULT_SETTINGS, saveTranscript: true }}
        onUpdateSettings={onUpdateSettings}
        initialized
        status="idle"
        onResetStats={vi.fn()}
        onRerunSetup={vi.fn()}
        accessibilityGranted
        onCheckForUpdate={vi.fn(async () => {})}
        onDownloadUpdate={vi.fn()}
        onRestartUpdate={vi.fn()}
        onOpenUpdate={vi.fn()}
        updateStatus={{ phase: 'idle' }}
        configureError={null}
      />,
    ));
    const delivery = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Delivery',
    ) as HTMLButtonElement;
    await act(async () => delivery.click());
    const autoPasteSwitch = container.querySelector('[role="switch"][aria-label="Auto paste"]') as HTMLElement;
    expect(autoPasteSwitch).toBeTruthy();
    expect(autoPasteSwitch.hasAttribute('data-disabled')).toBe(true);
    onUpdateSettings.mockClear();
    await act(async () => autoPasteSwitch.click());
    expect(onUpdateSettings).not.toHaveBeenCalled();
  });

  it('shows Smart destination only on macOS and preserves an enabled preference while unavailable', async () => {
    const ownUserAgent = Object.getOwnPropertyDescriptor(navigator, 'userAgent');
    try {
      Object.defineProperty(navigator, 'userAgent', {
        configurable: true,
        value: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 15_0) AppleWebKit/605.1.15',
      });
      expect(isMacOSSettingsPlatform()).toBe(true);

      await act(async () => renderPanel(true, {
        ...DEFAULT_SETTINGS,
        autoPaste: true,
        smartDestinationEnabled: true,
        saveTranscript: true,
      }));
      const delivery = Array.from(container.querySelectorAll('nav button')).find(
        (button) => button.textContent === 'Delivery',
      ) as HTMLButtonElement;
      await act(async () => delivery.click());

      let smartDestination = container.querySelector(
        '[role="switch"][aria-label="Smart destination"]',
      ) as HTMLElement;
      expect(smartDestination).toBeTruthy();
      expect(smartDestination.getAttribute('aria-checked')).toBe('true');
      expect(smartDestination.hasAttribute('data-disabled')).toBe(true);
      expect(container.textContent).toContain(
        'Keep your recent text field after media controls or switching to an app without one',
      );
      onUpdateSettings.mockClear();
      await act(async () => smartDestination.click());
      expect(onUpdateSettings).not.toHaveBeenCalled();

      await act(async () => renderPanel(true, {
        ...DEFAULT_SETTINGS,
        autoPaste: true,
        smartDestinationEnabled: true,
      }));
      smartDestination = container.querySelector(
        '[role="switch"][aria-label="Smart destination"]',
      ) as HTMLElement;
      expect(smartDestination.hasAttribute('data-disabled')).toBe(false);
      await act(async () => smartDestination.click());
      expect(onUpdateSettings).toHaveBeenCalledWith({ smartDestinationEnabled: false });

      Object.defineProperty(navigator, 'userAgent', {
        configurable: true,
        value: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36',
      });
      expect(isMacOSSettingsPlatform()).toBe(false);
      await act(async () => renderPanel(true, {
        ...DEFAULT_SETTINGS,
        autoPaste: true,
        smartDestinationEnabled: true,
      }));
      expect(container.querySelector('[aria-label="Smart destination"]')).toBeNull();

      const search = container.querySelector('input[type="search"]') as HTMLInputElement;
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
      await act(async () => {
        setter.call(search, 'smart destination');
        search.dispatchEvent(new Event('input', { bubbles: true }));
      });
      expect(container.textContent).toContain('No settings match “smart destination”.');
    } finally {
      if (ownUserAgent) Object.defineProperty(navigator, 'userAgent', ownUserAgent);
      else Reflect.deleteProperty(navigator, 'userAgent');
    }
  });

  it('reveals dependent controls directly beneath their owning switch', async () => {
    function Harness() {
      const [current, setCurrent] = useState(DEFAULT_SETTINGS);
      return (
        <SettingsPanel
          settings={current}
          onUpdateSettings={(updates) => setCurrent((previous) => ({ ...previous, ...updates }))}
          initialized
          status="idle"
          onResetStats={vi.fn()}
          onRerunSetup={vi.fn()}
          accessibilityGranted={false}
          onCheckForUpdate={vi.fn(async () => {})}
        onDownloadUpdate={vi.fn()}
        onRestartUpdate={vi.fn()}
        onOpenUpdate={vi.fn()}
          updateStatus={{ phase: 'idle' }}
          configureError={null}
        />
      );
    }

    await act(async () => root.render(<Harness />));
    const delivery = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Delivery',
    ) as HTMLButtonElement;
    await act(async () => delivery.click());

    const autoPasteSwitch = container.querySelector('[role="switch"][aria-label="Auto paste"]') as HTMLElement;
    const autoPasteBranch = autoPasteSwitch.closest('.settings-setting-row')?.nextElementSibling as HTMLElement;
    expect(autoPasteBranch.dataset.expanded).toBe('false');
    expect(autoPasteBranch.getAttribute('aria-hidden')).toBe('true');

    await act(async () => autoPasteSwitch.click());
    expect(autoPasteBranch.dataset.expanded).toBe('true');
    expect(autoPasteBranch.getAttribute('aria-hidden')).toBe('false');
    expect(autoPasteBranch.hasAttribute('inert')).toBe(false);
    expect(autoPasteBranch.querySelector('input[type="range"]')).toBeTruthy();
    expect(autoPasteBranch.textContent).toContain('Accessibility permission is required');
    expect(autoPasteBranch.textContent).not.toContain('Accessibility permission granted');

    const autoPasteSlider = autoPasteBranch.querySelector('input[type="range"]') as HTMLInputElement;
    autoPasteSlider.focus();
    await act(async () => autoPasteSwitch.click());
    expect(autoPasteBranch.dataset.expanded).toBe('false');
    expect(autoPasteBranch.getBoundingClientRect().height).toBe(0);
    expect(autoPasteBranch.hasAttribute('inert')).toBe(true);
    expect(autoPasteBranch.querySelector('input[type="range"]')).toBeTruthy();
    expect(document.activeElement).toBe(autoPasteSwitch);

    const pasteLast = container.querySelector('[data-setting-target="paste-last-shortcut"]') as HTMLElement;
    expect(pasteLast.parentElement).toBe(autoPasteBranch.parentElement);
    expect(autoPasteBranch.nextElementSibling).toBe(pasteLast);
  });

  it('uses the same branch treatment for cleanup and file output dependencies', async () => {
    await act(async () => root.render(
      <SettingsPanel
        settings={{ ...DEFAULT_SETTINGS, cleanupEnabled: true, saveAudio: true }}
        onUpdateSettings={onUpdateSettings}
        initialized
        status="idle"
        onResetStats={vi.fn()}
        onRerunSetup={vi.fn()}
        accessibilityGranted
        onCheckForUpdate={vi.fn(async () => {})}
        onDownloadUpdate={vi.fn()}
        onRestartUpdate={vi.fn()}
        onOpenUpdate={vi.fn()}
        updateStatus={{ phase: 'idle' }}
        configureError={null}
      />,
    ));
    const text = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Text & vocabulary',
    ) as HTMLButtonElement;
    await act(async () => text.click());
    const cleanupSwitch = container.querySelector('[role="switch"][aria-label="Transcript cleanup"]') as HTMLElement;
    const cleanupBranch = cleanupSwitch.closest('.settings-setting-row')?.nextElementSibling as HTMLElement;
    expect(cleanupBranch.classList.contains('settings-dependent-branch')).toBe(true);
    expect(cleanupBranch.dataset.expanded).toBe('true');
    expect(cleanupBranch.querySelector('[aria-label="Remove filler words"]')).toBeTruthy();

    await act(async () => {
      const delivery = Array.from(container.querySelectorAll('nav button')).find(
        (button) => button.textContent === 'Delivery',
      ) as HTMLButtonElement;
      delivery.click();
    });
    const fileOutput = container.querySelector('[data-setting-target="file-output"]') as HTMLElement;
    const fileBranch = fileOutput.nextElementSibling?.nextElementSibling as HTMLElement;
    expect(fileBranch.dataset.expanded).toBe('true');
    expect(fileBranch.textContent).toContain('output folder');
  });
});

describe('effectiveAutoPaste', () => {
  it('preserves the preference while pausing delivery for either file output', () => {
    expect(effectiveAutoPaste({ autoPaste: true, saveTranscript: false, saveAudio: false })).toBe(true);
    expect(effectiveAutoPaste({ autoPaste: true, saveTranscript: true, saveAudio: false })).toBe(false);
    expect(effectiveAutoPaste({ autoPaste: true, saveTranscript: false, saveAudio: true })).toBe(false);
  });

  it('describes paused and already-off preferences without conflating them', () => {
    expect(autoPasteDeliveryDescription({ autoPaste: true, saveTranscript: true, saveAudio: false })).toContain('Paused');
    expect(fileOutputDeliveryDescription({ autoPaste: true })).toContain('paused');

    expect(autoPasteDeliveryDescription({ autoPaste: false, saveTranscript: false, saveAudio: true })).toBe(
      'Unavailable while saving to file',
    );
    expect(fileOutputDeliveryDescription({ autoPaste: false })).toBe(
      'Auto-paste stays off.',
    );
  });
});
