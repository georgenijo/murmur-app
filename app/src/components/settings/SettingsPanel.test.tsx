import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS, type Settings } from '../../lib/settings';
import { CUSTOM_QUERY_PRESET } from '../../lib/queryProviders';
import type { TransformModelStatus } from '../../lib/transformSettings';
import {
  SETTINGS_CATEGORIES,
  SettingsPanel,
  autoPasteDeliveryDescription,
  effectiveAutoPaste,
  fileOutputDeliveryDescription,
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
vi.mock('./VoiceCommandsManager', () => ({ VoiceCommandsManager: () => <div>Voice commands editor</div> }));
vi.mock('./TransformsManager', () => ({ TransformsManager: () => <div>Transforms manager</div> }));
vi.mock('./VocabScanStrip', () => ({ VocabScanStrip: () => <div>Vocabulary scan</div> }));

const transformMocks = vi.hoisted(() => ({
  status: null as TransformModelStatus | null,
  setTransformKey: vi.fn(async () => {}),
  startTransformListener: vi.fn(async () => {}),
}));
vi.mock('../../lib/transformSettings', () => ({
  TRANSFORM_MODEL_SIZE_LABEL: '1.1 GB',
  transformModelStatus: vi.fn(async () => transformMocks.status),
  downloadTransformModel: vi.fn(async () => {}),
  removeTransformModel: vi.fn(async () => {}),
  resetTransformRuntime: vi.fn(async () => {}),
  setTransformKey: transformMocks.setTransformKey,
  startTransformListener: transformMocks.startTransformListener,
  stopTransformListener: vi.fn(async () => {}),
}));

beforeEach(() => {
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
  coreMocks.invoke.mockImplementation(async (command: string) => {
    if (command === 'get_audio_input_inventory') return {
      schemaVersion: 1,
      revision: 1,
      status: 'available',
      devices: [],
      defaultInputId: null,
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
  const onUpdateSettings = vi.fn();

  function renderPanel(isOpen = true) {
    void isOpen;
    return root.render(
      <SettingsPanel
        settings={DEFAULT_SETTINGS}
        onUpdateSettings={onUpdateSettings}
        initialized
        status="idle"
        onResetStats={vi.fn()}
        onRerunSetup={vi.fn()}
        accessibilityGranted
        onCheckForUpdate={vi.fn(async () => {})}
        updateStatus={{ phase: 'idle' }}
        configureError={null}
      />,
    );
  }

  beforeEach(async () => {
    scrollTo.mockReset();
    onUpdateSettings.mockReset();
    Object.defineProperty(HTMLElement.prototype, 'scrollTo', { value: scrollTo, configurable: true });
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { value: vi.fn(), configurable: true });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => renderPanel());
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('renders direct settings destinations with General selected first', () => {
    expect(SETTINGS_CATEGORIES.map((category) => category.label)).toEqual([
      'General', 'Recording', 'Delivery', 'Meetings', 'Text & Vocabulary', 'AI & Models', 'Appearance',
    ]);
    const nav = container.querySelector('nav[aria-label="Settings pages"]') as HTMLElement;
    expect(Array.from(nav.querySelectorAll('button')).map((button) => button.textContent)).toEqual(SETTINGS_CATEGORIES.map((category) => category.label));
    expect(nav.querySelector('[aria-current="page"]')?.textContent).toBe('General');
    expect(container.querySelector('h1')?.textContent).toBe('General');
    expect(container.textContent).toContain('Launch at Login');
  });

  it('commits keyboard changes to voice-detection sensitivity', async () => {
    const recording = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Recording',
    ) as HTMLButtonElement;
    await act(async () => recording.click());
    const heading = Array.from(container.querySelectorAll('p')).find(
      (item) => item.textContent === 'Voice Detection',
    ) as HTMLParagraphElement;
    const slider = heading.parentElement?.querySelector('input[type="range"]') as HTMLInputElement;
    await act(async () => {
      slider.value = '75';
      slider.dispatchEvent(new Event('input', { bubbles: true }));
      slider.dispatchEvent(new KeyboardEvent('keyup', { key: 'ArrowRight', bubbles: true }));
    });

    expect(onUpdateSettings).toHaveBeenCalledWith({ vadSensitivity: 75 });
  });

  it('updates and previews recording sound cues', async () => {
    const recording = Array.from(container.querySelectorAll('nav button')).find(
      (button) => button.textContent === 'Recording',
    ) as HTMLButtonElement;
    await act(async () => recording.click());
    const soundSwitch = container.querySelector('button[aria-label="Sound Cues"]') as HTMLButtonElement;
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
    const meetingSwitch = container.querySelector('button[aria-label="Meeting Cues"]') as HTMLButtonElement;
    await act(async () => meetingSwitch.click());
    expect(onUpdateSettings).toHaveBeenCalledWith({ meetingSoundCuesEnabled: true });
  });

  it('hides the NotchPill setting when the companion app is absent', () => {
    expect(container.textContent).not.toContain('Mirror Captions to NotchPill');
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
        schemaVersion: 1,
        revision: 2,
        status: 'unavailable',
        devices: [],
        defaultInputId: null,
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

    expect(container.textContent).toContain('Mirror Captions to NotchPill');
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
    expect(container.textContent).toContain('Mirror Captions to NotchPill');

    coreMocks.notchPillDetectionError = true;
    await act(async () => {
      window.dispatchEvent(new Event('focus'));
      await Promise.resolve();
    });

    expect(container.textContent).not.toContain('Mirror Captions to NotchPill');
  });

  it('opens focused Recording, Meetings, Text, and AI destinations', async () => {
    for (const [page, expected] of [
      ['Recording', 'Voice Detection'],
      ['Meetings', 'Keep Meeting Audio'],
      ['Text & Vocabulary', 'Vocabulary'],
      ['AI & Models', 'Speech-to-Text'],
    ] as const) {
      const button = Array.from(container.querySelectorAll('nav button')).find((item) => item.textContent === page) as HTMLButtonElement;
      await act(async () => button.click());
      expect(container.textContent).toContain(expected);
      expect(button.getAttribute('aria-current')).toBe('page');
    }
    expect(scrollTo).toHaveBeenCalledWith({ top: 0 });
  });

  it('opens Voice Query as a focused AI & Models drill-down', async () => {
    const button = Array.from(container.querySelectorAll('nav button')).find((item) => item.textContent === 'AI & Models') as HTMLButtonElement;
    await act(async () => button.click());
    const voiceQuery = Array.from(container.querySelectorAll('button')).find(
      (item) => item.textContent?.includes('Voice Query') && item.textContent?.includes('Configure'),
    ) as HTMLButtonElement;
    await act(async () => voiceQuery.click());

    expect(container.textContent).toContain('Voice Query');
    expect(container.textContent).toContain('may send the question or answer to cloud services');
    expect(container.textContent).toContain('No shell is ever invoked');
    expect(container.textContent).toContain('Context shared with the CLI');
    expect(container.textContent).toContain('Off by default');
    expect(container.textContent).toContain('never auto-pasted');
    const autoCopyToggle = container.querySelector(
      '[role="switch"][aria-label="Automatically copy answers"]',
    ) as HTMLButtonElement;
    expect(autoCopyToggle.getAttribute('aria-checked')).toBe('true');
    await act(async () => autoCopyToggle.click());
    expect(onUpdateSettings).toHaveBeenCalledWith({ queryAutomaticallyCopyAnswers: false });
    const historyToggle = container.querySelector(
      '[role="switch"][aria-label="Keep Voice Query history on this Mac"]',
    ) as HTMLButtonElement;
    expect(historyToggle.getAttribute('aria-checked')).toBe('false');
    await act(async () => historyToggle.click());
    expect(onUpdateSettings).toHaveBeenCalledWith({ retainQueryHistory: true });
    expect(container.textContent).toContain('This includes queries that shared app context');
    expect(container.textContent).toContain('a saved answer may quote it');
    expect(container.textContent).toContain('a saved answer may quote context shared with the CLI');
    expect(container.textContent).not.toContain('Context content never enters history');
  });

  it('opens editors as a Text settings drill-down with explicit back navigation', async () => {
    const settingsPages = container.querySelector('nav[aria-label="Settings pages"]') as HTMLElement;
    const textTab = Array.from(settingsPages.querySelectorAll('button')).find(
      (button) => button.textContent === 'Text & Vocabulary',
    ) as HTMLButtonElement;
    await act(async () => textTab.click());

    const aliases = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.trim().startsWith('Aliases'),
    ) as HTMLButtonElement;
    await act(async () => aliases.click());

    expect(settingsPages.isConnected).toBe(true);
    expect(settingsPages.querySelector('[aria-current="page"]')?.textContent).toBe('Text & Vocabulary');
    expect(container.querySelector('nav[aria-label="Settings editors"]')).toBeNull();
    expect(container.querySelector('h1')?.textContent).toBe('Aliases');

    const back = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.includes('Back to Text settings'),
    ) as HTMLButtonElement;
    expect(back).toBeDefined();
    await act(async () => back.click());

    expect(container.textContent).toContain('Text & Vocabulary');
    expect(container.querySelector('[aria-labelledby="settings-editor-title"]')).toBeNull();
    expect(settingsPages.querySelector('[aria-current="page"]')?.textContent).toBe('Text & Vocabulary');
  });

  it('accepts precise sidebar destinations for an editor or setting target', async () => {
    const renderRequest = (pageRequest: { page: string; token: number; editorTab?: 'commands'; target?: string }) => root.render(
      <SettingsPanel
        settings={DEFAULT_SETTINGS}
        onUpdateSettings={onUpdateSettings}
        initialized
        status="idle"
        onResetStats={vi.fn()}
        onRerunSetup={vi.fn()}
        accessibilityGranted
        onCheckForUpdate={vi.fn(async () => {})}
        updateStatus={{ phase: 'idle' }}
        configureError={null}
        pageRequest={pageRequest}
      />,
    );

    await act(async () => renderRequest({ page: 'text', editorTab: 'commands', token: 1 }));
    expect(container.querySelector('h1')?.textContent).toBe('Voice Commands');
    expect(container.textContent).toContain('Voice commands editor');

    await act(async () => renderRequest({ page: 'delivery', target: 'app-overrides', token: 2 }));
    expect(container.querySelector('h1')?.textContent).toBe('Delivery');
    expect(container.textContent).toContain('App overrides editor');
  });

  it('returns from an editor with Escape', async () => {
    const settingsPages = container.querySelector('nav[aria-label="Settings pages"]') as HTMLElement;
    const textTab = Array.from(settingsPages.querySelectorAll('button')).find(
      (button) => button.textContent === 'Text & Vocabulary',
    ) as HTMLButtonElement;
    await act(async () => textTab.click());
    const vocabulary = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.trim().startsWith('Vocabulary'),
    ) as HTMLButtonElement;
    await act(async () => vocabulary.click());

    expect(container.querySelector('h1')?.textContent).toBe('Vocabulary');
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' })));
    expect(container.querySelector('[aria-labelledby="settings-editor-title"]')).toBeNull();
    expect(settingsPages.querySelector('[aria-current="page"]')?.textContent).toBe('Text & Vocabulary');
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
    const input = container.querySelector('input[placeholder="Search Settings"]') as HTMLInputElement;
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

  it('finds the Voice Query clipboard preference from settings search', async () => {
    const input = container.querySelector('input[placeholder="Search Settings"]') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(input, 'clipboard copy');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });

    const result = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.includes('Voice Query'),
    ) as HTMLButtonElement;
    expect(result).toBeDefined();
    await act(async () => result.click());
    expect(container.querySelector('nav [aria-current="page"]')?.textContent).toBe('AI & Models');
    expect(container.querySelector(
      '[role="switch"][aria-label="Automatically copy answers"]',
    )).not.toBeNull();
  });

  it('opens diagnostics when the cross-tab result explicitly targets them', async () => {
    const input = container.querySelector('input[placeholder="Search Settings"]') as HTMLInputElement;
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
});

describe('SettingsPanel Voice Query async ownership', () => {
  let container: HTMLDivElement;
  let root: Root;
  let currentSettings: Settings;
  let updateSettings!: (updates: Partial<Settings>) => void;

  const previewStatus = {
    previewId: null,
    state: 'idle',
    stillConnecting: false,
    errorKind: null,
    message: null,
  };

  const idleInvoke = async (command: string) => {
    if (command === 'list_audio_devices' || command === 'load_query_environment') return [];
    if (command === 'get_microphone_preview_status' || command === 'stop_microphone_preview') {
      return previewStatus;
    }
    if (command === 'cancel_microphone_preview') return false;
    if (command === 'list_query_provider_presets') return [];
    return undefined;
  };

  function installVoiceQueryInvoke(commandName: string, response: Promise<unknown>) {
    coreMocks.invoke.mockImplementation((command: string) => {
      if (command === commandName) return response;
      return idleInvoke(command);
    });
  }

  beforeEach(() => {
    coreMocks.invoke.mockImplementation(idleInvoke);
    Object.defineProperty(HTMLElement.prototype, 'scrollTo', { value: vi.fn(), configurable: true });
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { value: vi.fn(), configurable: true });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  async function renderVoiceQuery(overrides: Partial<Settings> = {}) {
    function Harness() {
      const [settings, setSettings] = useState<Settings>({
        ...DEFAULT_SETTINGS,
        queryProvider: 'custom',
        queryExecutable: '/usr/bin/printf',
        queryArguments: ['%s'],
        queryHotkey: null,
        ...overrides,
      });
      currentSettings = settings;
      updateSettings = (updates) => setSettings((current) => ({ ...current, ...updates }));
      return (
        <SettingsPanel
          settings={settings}
          onUpdateSettings={updateSettings}
          initialized
          status="idle"
          onResetStats={vi.fn()}
          onRerunSetup={vi.fn()}
          accessibilityGranted
          onCheckForUpdate={vi.fn(async () => {})}
          updateStatus={{ phase: 'idle' }}
          configureError={null}
          pageRequest={{ page: 'voice-query', token: 1 }}
        />
      );
    }
    await act(async () => {
      root.render(<Harness />);
      await Promise.resolve();
    });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  async function chooseProvider(label: string) {
    const provider = Array.from(container.querySelectorAll('[role="combobox"]')).find(
      (element) => element.textContent?.trim() === currentSettings.queryProvider.charAt(0).toUpperCase()
        + currentSettings.queryProvider.slice(1),
    ) as HTMLButtonElement;
    await act(async () => provider.click());
    const option = Array.from(container.querySelectorAll('[role="option"]')).find(
      (element) => element.textContent?.includes(label),
    ) as HTMLLIElement;
    await act(async () => {
      option.click();
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  it('does not enable an edited command after an older validation resolves', async () => {
    let resolveValidation!: () => void;
    const validation = new Promise<void>((resolve) => { resolveValidation = resolve; });
    installVoiceQueryInvoke('validate_query_command', validation);
    await renderVoiceQuery();

    const enable = container.querySelector('[role="switch"][aria-label="Enable Voice Query"]') as HTMLButtonElement;
    await act(async () => {
      enable.click();
      await Promise.resolve();
    });
    const executable = container.querySelector('#query-executable') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(executable, '/usr/bin/false');
      executable.dispatchEvent(new Event('input', { bubbles: true }));
      await Promise.resolve();
    });
    await act(async () => {
      resolveValidation();
      await validation;
      await Promise.resolve();
    });

    expect(currentSettings.queryExecutable).toBe('/usr/bin/false');
    expect(currentSettings.queryHotkey).toBeNull();
    expect(enable.getAttribute('aria-checked')).toBe('false');
  });

  it('shows custom-provider setup guidance and enable errors beside the toggle', async () => {
    await renderVoiceQuery({ queryExecutable: '', queryArguments: [] });

    expect(container.textContent).toContain('For a local smoke test');
    expect(container.textContent).toContain('/usr/bin/printf');
    const enable = container.querySelector(
      '[role="switch"][aria-label="Enable Voice Query"]',
    ) as HTMLButtonElement;
    await act(async () => enable.click());

    const alert = container.querySelector('[role="alert"]') as HTMLParagraphElement;
    const executable = container.querySelector('#query-executable') as HTMLInputElement;
    expect(alert.textContent).toContain('Choose the absolute path');
    expect(alert.compareDocumentPosition(executable) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(currentSettings.queryHotkey).toBeNull();
  });

  it('keeps the same shortcut after the exact new preset validates and passes preflight', async () => {
    coreMocks.invoke.mockImplementation((command: string) => {
      if (command === 'list_query_provider_presets') {
        return Promise.resolve([
          {
            id: 'custom',
            label: 'Custom',
            discoveryPaths: [],
            discoveredExecutable: null,
            recommendedArguments: [],
            authProbeArguments: [],
            authFailureSignatures: [],
            signInArguments: [],
            signInFix: null,
            permittedEnvironmentVariables: [],
          },
          {
            id: 'cursor',
            label: 'Cursor',
            discoveryPaths: [],
            discoveredExecutable: '/usr/local/bin/cursor-agent',
            recommendedArguments: ['--print', '--mode', 'ask', '--single-turn', '--trust'],
            authProbeArguments: ['status'],
            authFailureSignatures: [],
            signInArguments: ['login'],
            signInFix: 'Run cursor-agent login in Terminal.',
            permittedEnvironmentVariables: [],
          },
        ]);
      }
      if (command === 'validate_query_command') return Promise.resolve();
      if (command === 'test_query_provider') {
        return Promise.resolve({
          ok: true,
          authenticated: true,
          errorCode: null,
          stdout: 'ready',
          stderr: '',
          stdoutTruncated: false,
          stderrTruncated: false,
          signInFix: null,
        });
      }
      return idleInvoke(command);
    });
    await renderVoiceQuery({ queryHotkey: 'ctrl_l' });

    await chooseProvider('Cursor');
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(currentSettings.queryProvider).toBe('cursor');
    expect(currentSettings.queryHotkey).toBe('ctrl_l');
    expect(container.textContent).toContain('Voice Query stayed enabled after validation and preflight');
    const expectedCommand = {
      provider: 'cursor',
      executable: '/usr/local/bin/cursor-agent',
      arguments: ['--print', '--mode', 'ask', '--single-turn', '--trust'],
      timeoutSeconds: DEFAULT_SETTINGS.queryTimeoutSeconds,
      contextLevel: DEFAULT_SETTINGS.queryContextLevel,
      retainQueryHistory: DEFAULT_SETTINGS.retainQueryHistory,
    };
    expect(coreMocks.invoke).toHaveBeenCalledWith('validate_query_command', {
      command: expectedCommand,
    });
    expect(coreMocks.invoke).toHaveBeenCalledWith('test_query_provider', {
      command: expectedCommand,
    });
  });

  it('leaves Voice Query disabled with an actionable error when the new preflight fails', async () => {
    coreMocks.invoke.mockImplementation((command: string) => {
      if (command === 'list_query_provider_presets') {
        return Promise.resolve([
          CUSTOM_QUERY_PRESET,
          {
            ...CUSTOM_QUERY_PRESET,
            id: 'cursor',
            label: 'Cursor',
            discoveredExecutable: '/usr/local/bin/cursor-agent',
            recommendedArguments: ['--print', '--mode', 'ask', '--single-turn', '--trust'],
            authProbeArguments: ['status'],
            signInArguments: ['login'],
            signInFix: 'Run cursor-agent login in Terminal.',
          },
        ]);
      }
      if (command === 'validate_query_command') return Promise.resolve();
      if (command === 'test_query_provider') {
        return Promise.resolve({
          ok: false,
          authenticated: false,
          errorCode: 'provider_not_authenticated',
          stdout: '',
          stderr: 'Not logged in',
          stdoutTruncated: false,
          stderrTruncated: false,
          signInFix: 'Run cursor-agent login in Terminal.',
        });
      }
      return idleInvoke(command);
    });
    await renderVoiceQuery({ queryHotkey: 'ctrl_l' });

    await chooseProvider('Cursor');
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(currentSettings.queryProvider).toBe('cursor');
    expect(currentSettings.queryHotkey).toBeNull();
    expect(container.textContent).toContain('Voice Query remains off. Run cursor-agent login in Terminal.');
  });

  it('does not auto-enable or preflight a provider switch that started disabled', async () => {
    coreMocks.invoke.mockImplementation((command: string) => {
      if (command === 'list_query_provider_presets') {
        return Promise.resolve([
          CUSTOM_QUERY_PRESET,
          {
            ...CUSTOM_QUERY_PRESET,
            id: 'cursor',
            label: 'Cursor',
            discoveredExecutable: '/usr/local/bin/cursor-agent',
            recommendedArguments: ['--print', '--mode', 'ask', '--single-turn', '--trust'],
          },
        ]);
      }
      return idleInvoke(command);
    });
    await renderVoiceQuery({ queryHotkey: null });

    await chooseProvider('Cursor');

    expect(currentSettings.queryProvider).toBe('cursor');
    expect(currentSettings.queryHotkey).toBeNull();
    expect(coreMocks.invoke).not.toHaveBeenCalledWith(
      'validate_query_command',
      expect.anything(),
    );
    expect(coreMocks.invoke).not.toHaveBeenCalledWith(
      'test_query_provider',
      expect.anything(),
    );
  });

  it('lets only the latest rapid provider switch restore the carried shortcut', async () => {
    let resolveCursor!: (value: Record<string, unknown>) => void;
    let resolveClaude!: (value: Record<string, unknown>) => void;
    const cursorResult = new Promise<Record<string, unknown>>((resolve) => { resolveCursor = resolve; });
    const claudeResult = new Promise<Record<string, unknown>>((resolve) => { resolveClaude = resolve; });
    coreMocks.invoke.mockImplementation((command: string, args?: { command?: { provider?: string } }) => {
      if (command === 'list_query_provider_presets') {
        return Promise.resolve([
          CUSTOM_QUERY_PRESET,
          {
            ...CUSTOM_QUERY_PRESET,
            id: 'cursor',
            label: 'Cursor',
            discoveredExecutable: '/usr/local/bin/cursor-agent',
            recommendedArguments: ['--trust'],
          },
          {
            ...CUSTOM_QUERY_PRESET,
            id: 'claude',
            label: 'Claude',
            discoveredExecutable: '/usr/local/bin/claude',
            recommendedArguments: ['--print'],
          },
        ]);
      }
      if (command === 'validate_query_command') return Promise.resolve();
      if (command === 'test_query_provider') {
        return args?.command?.provider === 'cursor' ? cursorResult : claudeResult;
      }
      return idleInvoke(command);
    });
    await renderVoiceQuery({ queryHotkey: 'ctrl_l' });

    await chooseProvider('Cursor');
    await chooseProvider('Claude');
    await act(async () => {
      resolveClaude({
        ok: true,
        authenticated: true,
        errorCode: null,
        stdout: '',
        stderr: '',
        stdoutTruncated: false,
        stderrTruncated: false,
        signInFix: null,
      });
      await claudeResult;
      await Promise.resolve();
    });
    expect(currentSettings.queryProvider).toBe('claude');
    expect(currentSettings.queryHotkey).toBe('ctrl_l');

    await act(async () => {
      resolveCursor({
        ok: true,
        authenticated: true,
        errorCode: null,
        stdout: 'STALE_CURSOR_RESULT',
        stderr: '',
        stdoutTruncated: false,
        stderrTruncated: false,
        signInFix: null,
      });
      await cursorResult;
      await Promise.resolve();
    });
    expect(currentSettings.queryProvider).toBe('claude');
    expect(currentSettings.queryHotkey).toBe('ctrl_l');
    expect(container.textContent).not.toContain('STALE_CURSOR_RESULT');
  });

  it('does not restore the shortcut when the preset command changes during deferred preflight', async () => {
    let resolvePreflight!: (value: Record<string, unknown>) => void;
    const preflight = new Promise<Record<string, unknown>>((resolve) => { resolvePreflight = resolve; });
    coreMocks.invoke.mockImplementation((command: string) => {
      if (command === 'list_query_provider_presets') {
        return Promise.resolve([
          CUSTOM_QUERY_PRESET,
          {
            ...CUSTOM_QUERY_PRESET,
            id: 'cursor',
            label: 'Cursor',
            discoveredExecutable: '/usr/local/bin/cursor-agent',
            recommendedArguments: ['--trust'],
          },
        ]);
      }
      if (command === 'validate_query_command') return Promise.resolve();
      if (command === 'test_query_provider') return preflight;
      return idleInvoke(command);
    });
    await renderVoiceQuery({ queryHotkey: 'ctrl_l' });

    await chooseProvider('Cursor');
    const executable = container.querySelector('#query-executable') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(executable, '/usr/bin/false');
      executable.dispatchEvent(new Event('input', { bubbles: true }));
      await Promise.resolve();
    });
    await act(async () => {
      resolvePreflight({
        ok: true,
        authenticated: true,
        errorCode: null,
        stdout: 'STALE_SUCCESS',
        stderr: '',
        stdoutTruncated: false,
        stderrTruncated: false,
        signInFix: null,
      });
      await preflight;
      await Promise.resolve();
    });

    expect(currentSettings.queryProvider).toBe('cursor');
    expect(currentSettings.queryExecutable).toBe('/usr/bin/false');
    expect(currentSettings.queryHotkey).toBeNull();
    expect(container.textContent).toContain(
      'Configuration changed during provider preflight. Voice Query remains off.',
    );
    expect(container.textContent).not.toContain('STALE_SUCCESS');
  });

  it('diagnoses an incomplete Codex platform package from its ENOENT probe output', async () => {
    installVoiceQueryInvoke('test_query_provider', Promise.resolve({
      ok: false,
      authenticated: false,
      errorCode: 'probe_failed',
      stdout: '',
      stderr: 'Error: spawn /opt/homebrew/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/codex/codex ENOENT',
      stdoutTruncated: false,
      stderrTruncated: false,
      signInFix: null,
    }));
    await renderVoiceQuery({
      queryProvider: 'codex',
      queryExecutable: '/opt/homebrew/bin/codex',
      queryArguments: ['exec', '--json'],
    });

    const testButton = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Test',
    ) as HTMLButtonElement;
    await act(async () => {
      testButton.click();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(container.textContent).toContain('The Codex CLI installation is incomplete');
    expect(container.textContent).not.toContain('/opt/homebrew');
    expect(container.textContent).not.toContain('ENOENT');
  });

  it('does not render a pending provider test under a newly selected provider', async () => {
    let resolveTest!: (result: Record<string, unknown>) => void;
    const pendingTest = new Promise<Record<string, unknown>>((resolve) => { resolveTest = resolve; });
    installVoiceQueryInvoke('test_query_provider', pendingTest);
    await renderVoiceQuery();

    const testButton = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Test',
    ) as HTMLButtonElement;
    await act(async () => {
      testButton.click();
      await Promise.resolve();
      updateSettings({
        queryProvider: 'codex',
        queryExecutable: '/opt/homebrew/bin/codex',
        queryArguments: ['exec'],
      });
      await Promise.resolve();
    });
    await act(async () => {
      resolveTest({
        ok: false,
        authenticated: false,
        errorCode: 'provider_not_authenticated',
        stdout: 'STALE_PROVIDER_OUTPUT',
        stderr: '',
        stdoutTruncated: false,
        stderrTruncated: false,
        signInFix: 'Run the old provider login.',
      });
      await pendingTest;
      await Promise.resolve();
    });

    expect(currentSettings.queryProvider).toBe('codex');
    expect(container.textContent).not.toContain('STALE_PROVIDER_OUTPUT');
    expect(container.textContent).not.toContain('Run the old provider login.');
  });

  it('exposes Clear as the recovery action after a corrupt environment load', async () => {
    coreMocks.invoke.mockImplementation((command: string) => {
      if (command === 'load_query_environment') return Promise.reject(new Error('invalid_environment'));
      if (command === 'save_query_environment') return Promise.resolve();
      return idleInvoke(command);
    });
    await renderVoiceQuery();
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(container.textContent).toContain('Clear saved values to repair it.');
    const clear = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Clear saved values',
    ) as HTMLButtonElement;
    expect(clear).toBeDefined();
    await act(async () => {
      clear.click();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(coreMocks.invoke).toHaveBeenCalledWith('save_query_environment', {
      provider: 'custom',
      variables: [],
    });
    expect(container.textContent).toContain('Saved config-directory values cleared.');
  });

  it('exposes corrupt-store recovery for a provider with no declared environment inputs', async () => {
    coreMocks.invoke.mockImplementation((command: string) => {
      if (command === 'list_query_provider_presets') {
        return Promise.resolve([{
          id: 'grok',
          label: 'Grok',
          discoveryPaths: [],
          discoveredExecutable: '/usr/bin/printf',
          recommendedArguments: ['%s'],
          authProbeArguments: ['models'],
          authFailureSignatures: [],
          signInArguments: ['login'],
          signInFix: 'Run grok login in Terminal.',
          permittedEnvironmentVariables: [],
        }]);
      }
      if (command === 'load_query_environment') return Promise.reject(new Error('invalid_environment'));
      if (command === 'save_query_environment') return Promise.resolve();
      return idleInvoke(command);
    });
    await renderVoiceQuery({ queryProvider: 'grok' });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(container.textContent).toContain('Clear saved values to repair it.');
    expect(container.querySelector('#query-env-CLAUDE_CONFIG_DIR')).toBeNull();
    expect(container.querySelector('#query-env-CODEX_HOME')).toBeNull();
    const clear = Array.from(container.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Clear saved values',
    ) as HTMLButtonElement;
    expect(clear).toBeDefined();
    await act(async () => {
      clear.click();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(coreMocks.invoke).toHaveBeenCalledWith('save_query_environment', {
      provider: 'grok',
      variables: [],
    });
    expect(container.textContent).toContain('Saved config-directory values cleared.');
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
      'Unavailable while file output is on. Turn off file output to enable auto-paste.',
    );
    expect(fileOutputDeliveryDescription({ autoPaste: false })).toBe(
      'Clipboard copying stays on; auto-paste remains off.',
    );
  });
});

describe('SettingsPanel transform block (#312 D1 round-2 findings 6-8)', () => {
  let container: HTMLDivElement;
  let root: Root;

  async function renderAndOpenTransform(settingsOverrides: Partial<typeof DEFAULT_SETTINGS> = {}) {
    Object.defineProperty(HTMLElement.prototype, 'scrollTo', { value: vi.fn(), configurable: true });
    Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', { value: vi.fn(), configurable: true });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(
      <SettingsPanel
        settings={{ ...DEFAULT_SETTINGS, ...settingsOverrides }}
        onUpdateSettings={vi.fn()}
        initialized
        status="idle"
        onResetStats={vi.fn()}
        onRerunSetup={vi.fn()}
        accessibilityGranted
        onCheckForUpdate={vi.fn(async () => {})}
        updateStatus={{ phase: 'idle' }}
        configureError={null}
        pageRequest={{ page: 'transform', token: 1 }}
      />,
    ));
    // Let the transformModelStatus() fetch effect resolve.
    await act(async () => {});
  }

  beforeEach(() => {
    transformMocks.status = null;
    transformMocks.setTransformKey.mockReset();
    transformMocks.startTransformListener.mockReset().mockResolvedValue(undefined);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('disables the Download button while the backend reports downloading (finding 6)', async () => {
    transformMocks.status = { state: 'downloading', path: null, sizeBytes: 100, sha256: 'x', runtimeDisabled: false };
    await renderAndOpenTransform();
    const downloadButton = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Working…') as HTMLButtonElement;
    expect(downloadButton).toBeTruthy();
    expect(downloadButton.disabled).toBe(true);
  });

  it('hides the Reset runtime button and notice when the breaker is not disabled (finding 7)', async () => {
    transformMocks.status = { state: 'ready', path: '/models/x', sizeBytes: 100, sha256: 'x', runtimeDisabled: false };
    await renderAndOpenTransform();
    expect(Array.from(container.querySelectorAll('button')).some((b) => b.textContent === 'Reset runtime')).toBe(false);
    expect(container.textContent).not.toContain('disabled after repeated faults');
  });

  it('shows the Reset runtime button and notice when runtimeDisabled is set (finding 7)', async () => {
    transformMocks.status = { state: 'ready', path: '/models/x', sizeBytes: 100, sha256: 'x', runtimeDisabled: true };
    await renderAndOpenTransform();
    expect(Array.from(container.querySelectorAll('button')).some((b) => b.textContent === 'Reset runtime')).toBe(true);
    expect(container.textContent).toContain('disabled after repeated faults');
  });

  it('renders shortcut-picker errors on their own line, not the model error slot (finding 8)', async () => {
    transformMocks.status = { state: 'ready', path: '/models/x', sizeBytes: 100, sha256: 'x', runtimeDisabled: false };
    transformMocks.setTransformKey.mockRejectedValue(new Error('shortcut already in use'));
    await renderAndOpenTransform({ transformHoldKey: 'alt_r' });

    const combobox = Array.from(container.querySelectorAll('button[role="combobox"]')).find(
      (button) => button.textContent === 'Right Option',
    ) as HTMLButtonElement;
    await act(async () => combobox.click());
    const option = Array.from(container.querySelectorAll('li[role="option"]')).find(
      (li) => li.textContent === 'Left Control',
    ) as HTMLLIElement;
    await act(async () => option.click());

    const errorParagraphs = Array.from(container.querySelectorAll('p')).filter((p) => p.className.includes('text-error'));
    expect(errorParagraphs).toHaveLength(1);
    expect(errorParagraphs[0].textContent).toContain('shortcut already in use');
  });
});
