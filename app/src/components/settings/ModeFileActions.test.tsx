import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ModeFileActions } from './ModeFileActions';
import { exportModeFile, type ModeExchangeSettings } from '../../lib/modeExchange';
import { BUILTIN_MODES } from '../../lib/settings';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn(), save: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: mocks.open, save: mocks.save }));
const mode = { ...BUILTIN_MODES[0], id: 'mode.focus', name: 'Focus', builtIn: false };
const empty: ModeExchangeSettings = { modes: [], appProfiles: [], browserSiteRules: [], siteModeLookupEnabled: false };
const source = { ...empty, modes: [mode] };

describe('Mode file actions', () => {
  let root: Root;
  let container: HTMLDivElement;
  const onChange = vi.fn();
  beforeEach(() => {
    vi.resetAllMocks();
    container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
    mocks.open.mockResolvedValue('/private/modes.json');
    mocks.invoke.mockResolvedValue(exportModeFile(source));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  const render = async (settings = empty) => { await act(async () => root.render(<ModeFileActions settings={settings} onChange={onChange} />)); };
  const button = (label: string) => {
    const found = [...container.querySelectorAll('button')].find((item) => item.textContent === label);
    if (!found) throw new Error(`Missing button: ${label}`);
    return found;
  };
  const click = async (label: string) => { await act(async () => button(label).click()); };

  it('previews without saving, then applies exactly one confirmed import', async () => {
    await render(); await click('Import Modes');
    expect(mocks.invoke).toHaveBeenCalledWith('read_modes_file', { path: '/private/modes.json' });
    expect(container.textContent).toContain('1 new Modes, 0 app bindings, 0 site rules');
    expect(onChange).not.toHaveBeenCalled();
    await click('Confirm import');
    expect(onChange).toHaveBeenCalledExactlyOnceWith(source);
    expect(container.textContent).toContain('Import applied');
  });
  it('cancels previews and native dialogs without mutation', async () => {
    await render(); await click('Import Modes'); await click('Cancel import');
    expect(onChange).not.toHaveBeenCalled();
    mocks.open.mockResolvedValue(null); await click('Import Modes');
    expect(container.querySelector('[aria-label="Mode import preview"]')).toBeNull();
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });
  it('blocks conflicts before mutation and displays exact duplicate counts', async () => {
    await render({ ...source, modes: [{ ...mode, name: 'Other' }] }); await click('Import Modes');
    expect(container.textContent).toContain('1 conflicts');
    expect(button('Confirm import').disabled).toBe(true);
    await click('Confirm import'); expect(onChange).not.toHaveBeenCalled();
    await render(source); await click('Import Modes');
    expect(container.textContent).toContain('1 exact duplicates skipped');
  });
  it('invalidates the preview after Settings changes', async () => {
    await render(); await click('Import Modes');
    await render({ ...empty, siteModeLookupEnabled: true });
    expect(button('Confirm import').disabled).toBe(true);
    expect(container.textContent).toContain('Choose the file again');
    await click('Confirm import'); expect(onChange).not.toHaveBeenCalled();
  });
  it('previews against current settings after the asynchronous read completes', async () => {
    let finish: (contents: string) => void = () => { throw new Error('Read not started'); };
    mocks.invoke.mockImplementation(() => new Promise<string>((resolve) => { finish = resolve; }));
    await render(); await click('Import Modes');
    await render({ ...source, modes: [{ ...mode, name: 'Changed while reading' }] });
    await act(async () => finish(exportModeFile(source)));
    expect(container.textContent).toContain('Mode ID conflict');
    expect(onChange).not.toHaveBeenCalled();
  });
  it('rejects malformed input before preview and never leaks file contents into errors', async () => {
    mocks.invoke.mockResolvedValue('{"private-transcript": broken}');
    await render(); await click('Import Modes');
    expect(container.querySelector('[role="alert"]')).not.toBeNull();
    expect(container.textContent).not.toContain('private-transcript');
    expect(onChange).not.toHaveBeenCalled();
  });
  it('saves a round-trippable custom-only export through the atomic sink', async () => {
    mocks.save.mockResolvedValue('/private/export.json');
    await render(source); await click('Export custom Modes');
    expect(mocks.invoke).toHaveBeenCalledWith('save_text_export', { path: '/private/export.json', contents: exportModeFile(source) });
    expect(container.textContent).toContain('Custom Modes exported');
    expect(onChange).not.toHaveBeenCalled();
  });
  it('shows file failures and cancels export without writes', async () => {
    mocks.invoke.mockRejectedValue('Mode files must be valid UTF-8.');
    await render(); await click('Import Modes');
    expect(container.textContent).toContain('could not read');
    mocks.invoke.mockClear(); mocks.save.mockResolvedValue(null);
    await click('Export custom Modes'); expect(mocks.invoke).not.toHaveBeenCalled();
    expect(onChange).not.toHaveBeenCalled();
  });
});
