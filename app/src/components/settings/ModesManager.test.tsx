import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ModesManager, previewModeText, summarizeMode } from './ModesManager';
import type { AppProfile, MurmurMode } from '../../lib/settings';

const profile: AppProfile = {
  bundleId: 'com.example.Editor', label: 'Editor', autoPasteOverride: null,
  cleanupOverride: null, smartFormattingOverride: null, cliFormattingOverride: null,
  writingStyle: null, ideContextEnabled: false, ideProjectRoots: [], queryContextExcluded: false,
};
const mode: MurmurMode = {
  id: 'mode.focus', name: 'Focus', builtIn: false, enabled: true,
  writingStyle: 'notes', cleanupEnabled: true, smartFormattingEnabled: true,
  cliFormattingEnabled: null, vocabularyPolicy: 'general', contextPolicy: 'none',
  modelId: null, language: null, autoPaste: false,
};

describe('ModesManager', () => {
  let root: Root;
  let container: HTMLDivElement;
  beforeEach(() => { container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container); });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

  it('summarizes behavior without retaining or injecting preview content', () => {
    expect(summarizeMode(mode)).toContain('general vocabulary');
    expect(previewModeText('um first new line second', mode)).toBe('first \n second');
  });

  it('binds one Mode to an application and supports disabling it', async () => {
    const onChange = vi.fn();
    await act(async () => root.render(<ModesManager modes={[mode]} profiles={[profile]} onChange={onChange} />));
    const focus = [...container.querySelectorAll('button')].find((button) => button.textContent?.includes('Focus'))!;
    await act(async () => focus.click());
    const checkbox = container.querySelector('input[type="checkbox"]') as HTMLInputElement;
    await act(async () => checkbox.click());
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      appProfiles: [expect.objectContaining({ modeId: 'mode.focus' })],
    }));
    const disable = [...container.querySelectorAll('button')].find((button) => button.textContent === 'Disable')!;
    await act(async () => disable.click());
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      modes: [expect.objectContaining({ enabled: false })],
    }));
  });
});
