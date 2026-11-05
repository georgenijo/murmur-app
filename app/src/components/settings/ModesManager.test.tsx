import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ModesManager, previewModeText, summarizeMode } from './ModesManager';
import type { AppProfile, MurmurMode } from '../../lib/settings';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

const profile: AppProfile = {
  bundleId: 'com.example.Editor', label: 'Editor', autoPasteOverride: null,
  cleanupOverride: null, smartFormattingOverride: null, cliFormattingOverride: null,
  writingStyle: null, ideContextEnabled: false, ideProjectRoots: [],
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

  it('marks the selected Mode row for keyboard and visual state styling', async () => {
    await act(async () => root.render(
      <ModesManager modes={[mode]} profiles={[]} siteLookupEnabled={false} siteRules={[]} onChange={vi.fn()} />,
    ));
    const rows = [...container.querySelectorAll<HTMLButtonElement>('[aria-label="Modes"] > button')];
    expect(rows.length).toBeGreaterThan(1);
    expect(rows[0].getAttribute('aria-pressed')).toBe('true');
    expect(rows[0].className).toContain('settings-mode-row');
    await act(async () => rows[1].click());
    expect(rows[0].getAttribute('aria-pressed')).toBe('false');
    expect(rows[1].getAttribute('aria-pressed')).toBe('true');
  });

  it('creates, tests, edits, and removes exact browser host rules behind global opt-in', async () => {
    const onChange = vi.fn();
    await act(async () => root.render(<ModesManager modes={[mode]} profiles={[]} siteLookupEnabled siteRules={[]} onChange={onChange} />));
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent?.includes('Focus'))!.click());
    const host = container.querySelector('[aria-label="Site rule host"]') as HTMLInputElement;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setValue.call(host, 'GitHub.com');
      host.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Add')!.click());
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      browserSiteRules: [expect.objectContaining({ host: 'github.com', modeId: 'mode.focus', enabled: true })],
    }));

    mocks.invoke.mockResolvedValue({ status: 'available', browserBundleId: 'com.apple.Safari', host: 'github.com' });
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent === 'Test current site')!.click());
    expect(container.textContent).toContain('Current site detected: github.com');
  });

  it('edits, disables, removes, and globally disables an existing rule', async () => {
    const onChange = vi.fn();
    const rule = { id: 'github', browserBundleId: 'com.apple.Safari', host: 'github.com', modeId: 'mode.focus', enabled: true };
    await act(async () => root.render(<ModesManager modes={[mode]} profiles={[]} siteLookupEnabled siteRules={[rule]} onChange={onChange} />));
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent?.includes('Focus'))!.click());

    const host = container.querySelector('[aria-label="Host for github.com"]') as HTMLInputElement;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setValue.call(host, 'docs.github.com');
      host.dispatchEvent(new FocusEvent('focusout', { bubbles: true }));
    });
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      browserSiteRules: [expect.objectContaining({ host: 'docs.github.com' })],
    }));
    await act(async () => (container.querySelector('[role="switch"][aria-label="Enable github.com"]') as HTMLElement).click());
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      browserSiteRules: [expect.objectContaining({ enabled: false })],
    }));
    await act(async () => (container.querySelector('[aria-label="Remove github.com…"]') as HTMLButtonElement).click());
    expect(onChange).not.toHaveBeenCalledWith(expect.objectContaining({ browserSiteRules: [] }));
    await act(async () => (container.querySelector('[aria-label="Confirm remove github.com…"]') as HTMLButtonElement).click());
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ browserSiteRules: [] }));
    await act(async () => (container.querySelector('[role="switch"][aria-label="Use browser site mode rules"]') as HTMLElement).click());
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ siteModeLookupEnabled: false }));
  });

  it('restores the saved host when an edit would duplicate another site rule', async () => {
    const onChange = vi.fn();
    const rules = [
      { id: 'github', browserBundleId: 'com.apple.Safari', host: 'github.com', modeId: 'mode.focus', enabled: true },
      { id: 'docs', browserBundleId: 'com.apple.Safari', host: 'docs.github.com', modeId: 'mode.focus', enabled: true },
    ];
    await act(async () => root.render(<ModesManager modes={[mode]} profiles={[]} siteLookupEnabled siteRules={rules} onChange={onChange} />));
    await act(async () => [...container.querySelectorAll('button')].find((button) => button.textContent?.includes('Focus'))!.click());
    const host = container.querySelector('[aria-label="Host for github.com"]') as HTMLInputElement;
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;

    await act(async () => {
      setValue.call(host, 'docs.github.com');
      host.dispatchEvent(new Event('input', { bubbles: true }));
      host.dispatchEvent(new FocusEvent('focusout', { bubbles: true }));
    });

    expect(host.value).toBe('github.com');
    expect(container.textContent).toContain('That browser and host already have a rule.');
    expect(onChange).not.toHaveBeenCalled();
  });

  it('binds one Mode to an application and supports disabling it', async () => {
    const onChange = vi.fn();
    const onModeBound = vi.fn();
    await act(async () => root.render(<ModesManager modes={[mode]} profiles={[profile]} siteLookupEnabled={false} siteRules={[]} onChange={onChange} onModeBound={onModeBound} />));
    const focus = [...container.querySelectorAll('button')].find((button) => button.textContent?.includes('Focus'))!;
    await act(async () => focus.click());
    const bindSwitch = container.querySelector('[role="switch"][aria-label="Bind Editor"]') as HTMLElement;
    await act(async () => bindSwitch.click());
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      appProfiles: [expect.objectContaining({ modeId: 'mode.focus' })],
    }));
    expect(onModeBound).toHaveBeenCalledOnce();
    const enabled = container.querySelector('[role="switch"][aria-label="Enable mode"]') as HTMLElement;
    expect(enabled.getAttribute('aria-checked')).toBe('true');
    await act(async () => enabled.click());
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      modes: [expect.objectContaining({ enabled: false })],
    }));
  });

  it('gives an empty app list a direct creation path', async () => {
    const onAddAppProfile = vi.fn();
    await act(async () => root.render(
      <ModesManager
        modes={[]}
        profiles={[]}
        siteLookupEnabled={false}
        siteRules={[]}
        onChange={vi.fn()}
        onAddAppProfile={onAddAppProfile}
      />,
    ));
    const addApp = Array.from(container.querySelectorAll('button'))
      .find((button) => button.textContent === 'Add an app') as HTMLButtonElement;
    expect(container.textContent).toContain('then choose its Mode here');
    await act(async () => addApp.click());
    expect(onAddAppProfile).toHaveBeenCalledWith('builtin.everyday', addApp);
  });
  it('groups the page into shared headed cards and deletes only after confirmation', async () => {
    const onChange = vi.fn();
    const rule = { id: 'github', browserBundleId: 'com.apple.Safari', host: 'github.com', modeId: 'mode.focus', enabled: true };
    await act(async () => root.render(<ModesManager modes={[mode]} profiles={[{ ...profile, modeId: 'mode.focus' }]} siteLookupEnabled={false} siteRules={[rule]} onChange={onChange} initialSelectedId="mode.focus" />));
    expect([...container.querySelectorAll('h2.settings-section-title')].map((heading) => heading.textContent))
      .toEqual(['Library', 'Selected mode', 'Where it applies', 'Before / after test']);
    expect(container.querySelectorAll('.settings-card.settings-rows')).toHaveLength(4);
    for (const label of ['Mode name', 'Writing style', 'Mode model', 'Mode language', 'Vocabulary policy', 'Use browser site mode rules']) {
      expect(container.querySelector(`[aria-label="${label}"]`)).not.toBeNull();
    }
    expect(container.querySelector('select, input[type="checkbox"]:not([aria-hidden="true"]), fieldset')).toBeNull();
    expect(container.querySelector('[data-setting-target="browser-sites"] [role="switch"]')).not.toBeNull();
    expect(container.querySelector('[data-setting-target="browser-site-rules"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="mode-preview"]')?.textContent).toBe('draft a concise project update');

    const button = (label: string) => [...container.querySelectorAll('button')].find((item) => item.textContent === label)!;
    await act(async () => button('Delete…').click());
    expect(onChange).not.toHaveBeenCalled();
    await act(async () => button('Confirm delete…').click());
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      modes: [], browserSiteRules: [], appProfiles: [expect.objectContaining({ modeId: null })],
    }));
  });
});
