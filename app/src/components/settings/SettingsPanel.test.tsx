import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS } from '../../lib/settings';
import type { TransformModelStatus } from '../../lib/transformSettings';
import {
  SETTINGS_CATEGORIES,
  SettingsPanel,
  autoPasteDeliveryDescription,
  effectiveAutoPaste,
  fileOutputDeliveryDescription,
} from './SettingsPanel';

vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '0.18.0') }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async (command: string) => command === 'list_audio_devices' ? [] : undefined) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
vi.mock('../../lib/modelRuntime', () => ({ useModelRuntimeCatalog: () => ({ models: [], byName: new Map(), error: null }) }));
vi.mock('../../lib/hooks/useVocabScan', () => ({
  useVocabScan: () => ({ status: 'idle', walker: null, stats: null, scan: vi.fn(), cancel: vi.fn() }),
}));
vi.mock('./AppOverridesEditor', () => ({ AppOverridesEditor: () => <div>App overrides editor</div> }));
vi.mock('./KnowledgeManager', () => ({ KnowledgeManager: () => <div>Knowledge manager</div> }));
vi.mock('./PerformanceLab', () => ({ PerformanceLab: () => <div>Performance lab</div> }));
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

describe('SettingsPanel information architecture', () => {
  let container: HTMLDivElement;
  let root: Root;
  const scrollTo = vi.fn();

  beforeEach(async () => {
    scrollTo.mockReset();
    Object.defineProperty(HTMLElement.prototype, 'scrollTo', { value: scrollTo, configurable: true });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(
      <SettingsPanel
        isOpen
        onClose={vi.fn()}
        settings={DEFAULT_SETTINGS}
        onUpdateSettings={vi.fn()}
        status="idle"
        onResetStats={vi.fn()}
        onViewLogs={vi.fn()}
        onRerunSetup={vi.fn()}
        accessibilityGranted
        onCheckForUpdate={vi.fn(async () => {})}
        updateStatus={{ phase: 'idle' }}
        configureError={null}
      />,
    ));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('renders ordered pages with Recording selected first (includes Transform)', () => {
    expect(SETTINGS_CATEGORIES.map((category) => category.label)).toEqual([
      'Recording', 'Transcription', 'Transform', 'Text & Vocabulary', 'Delivery', 'Performance', 'General',
    ]);
    const nav = container.querySelector('nav[aria-label="Settings pages"]') as HTMLElement;
    expect(Array.from(nav.querySelectorAll('button')).slice(1).map((button) => button.textContent)).toEqual(SETTINGS_CATEGORIES.map((category) => category.label));
    expect(nav.querySelector('[aria-current="page"]')?.textContent).toBe('Recording');
    expect(container.textContent).toContain('Microphone');
  });

  it('moves vocabulary, app overrides, Performance, and startup into their intended pages', async () => {
    for (const [page, expected] of [
      ['Text & Vocabulary', 'Names & Terms'],
      ['Delivery', 'Always copied to clipboard'],
      ['Performance', 'Performance lab'],
      ['General', 'Launch at Login'],
    ] as const) {
      const button = Array.from(container.querySelectorAll('nav button')).find((item) => item.textContent === page) as HTMLButtonElement;
      await act(async () => button.click());
      expect(container.textContent).toContain(expected);
      expect(button.getAttribute('aria-current')).toBe('page');
    }
    expect(scrollTo).toHaveBeenCalledWith({ top: 0 });
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
        isOpen
        onClose={vi.fn()}
        settings={{ ...DEFAULT_SETTINGS, ...settingsOverrides }}
        onUpdateSettings={vi.fn()}
        status="idle"
        onResetStats={vi.fn()}
        onViewLogs={vi.fn()}
        onRerunSetup={vi.fn()}
        accessibilityGranted
        onCheckForUpdate={vi.fn(async () => {})}
        updateStatus={{ phase: 'idle' }}
        configureError={null}
      />,
    ));
    const button = Array.from(container.querySelectorAll('nav button')).find((item) => item.textContent === 'Transform') as HTMLButtonElement;
    await act(async () => button.click());
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

    const combobox = container.querySelector('button[role="combobox"]') as HTMLButtonElement;
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
