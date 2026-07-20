import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS } from '../../lib/settings';
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
vi.mock('./VocabScanStrip', () => ({ VocabScanStrip: () => <div>Vocabulary scan</div> }));

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

  it('renders exactly six ordered pages with Recording selected first', () => {
    expect(SETTINGS_CATEGORIES.map((category) => category.label)).toEqual([
      'Recording', 'Transcription', 'Text & Vocabulary', 'Delivery', 'Performance', 'General',
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
