import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingToggle } from './SettingToggle';
import { SettingsBranch } from './SettingsBranch';
import { SettingsDisclosure, SettingsDisclosureContext } from './SettingsLayout';
import { readSettingsAdvanced, SETTINGS_ADVANCED_KEY, useSettingsAdvanced } from '../../lib/hooks/useSettingsAdvanced';

let container: HTMLDivElement;
let root: Root;
let reduced = false;
beforeEach(() => {
  vi.useFakeTimers();
  localStorage.clear();
  vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: reduced, addEventListener: vi.fn(), removeEventListener: vi.fn() })));
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); reduced = false; vi.useRealTimers(); vi.unstubAllGlobals(); });

function Preferences({ pane }: { pane: string }) {
  const [open, setOpen] = useSettingsAdvanced(pane);
  return <SettingsDisclosureContext.Provider value={{ open, setOpen }}>
    <SettingsDisclosure title="Advanced" layout="rows"><input aria-label="Rare preference" defaultValue="saved" /></SettingsDisclosure>
  </SettingsDisclosureContext.Provider>;
}

describe('real settings folds', () => {
  it('keeps dependent drafts mounted, hides closing controls from AX/Tab, and returns focus to the parent', async () => {
    function Form() {
      const [open, setOpen] = useState(true);
      return <><button onClick={() => setOpen(!open)}>Feature</button><SettingsBranch open={open}><input defaultValue="saved" /></SettingsBranch></>;
    }
    await act(async () => root.render(<Form />));
    const input = container.querySelector('input')!;
    input.value = 'draft'; input.focus();
    await act(async () => container.querySelector('button')!.click());
    expect(container.querySelector('[inert]')?.getAttribute('aria-hidden')).toBe('true');
    expect(document.activeElement).toBe(container.querySelector('button'));
    await act(async () => vi.advanceTimersByTime(500));
    expect(container.querySelector('input')).toBe(input);
    await act(async () => container.querySelector('button')!.click());
    expect(container.querySelector('[inert]')).toBeNull();
    expect(input.value).toBe('draft');
  });

  it('restores the parent switch instead of its preceding reset button', async () => {
    function Form() {
      const [open, setOpen] = useState(true);
      return <><SettingToggle title="Feature" description="A saved preference" checked={open} defaultChecked={false} onChange={() => setOpen(!open)} />
        <SettingsBranch open={open}><input /></SettingsBranch></>;
    }
    await act(async () => root.render(<Form />));
    container.querySelector('input')!.focus();
    const owner = container.querySelector('[role="switch"]') as HTMLButtonElement;
    await act(async () => owner.click());
    expect(document.activeElement).toBe(owner);
  });

  it('unmounts rare controls only after closing, restores focus, and cancels a reversed close', async () => {
    await act(async () => root.render(<Preferences pane="recording" />));
    const button = container.querySelector('button')!;
    expect(container.querySelector('input')).toBeNull();
    await act(async () => button.click());
    await act(async () => vi.advanceTimersByTime(20));
    const input = container.querySelector('input')!; input.focus();
    await act(async () => button.click());
    expect(document.activeElement).toBe(button);
    expect(input.closest('[inert][aria-hidden=true]')).not.toBeNull();
    await act(async () => vi.advanceTimersByTime(449));
    expect(input.isConnected).toBe(true);
    await act(async () => button.click());
    await act(async () => vi.advanceTimersByTime(500));
    expect(input.isConnected).toBe(true);
    await act(async () => button.click());
    await act(async () => vi.advanceTimersByTime(450));
    expect(container.querySelector('input')).toBeNull();
  });

  it('opens and unmounts instantly with Reduce Motion', async () => {
    reduced = true;
    await act(async () => root.render(<Preferences pane="general" />));
    await act(async () => container.querySelector('button')!.click());
    expect(container.querySelector('.settings-dependent-branch-open input')).not.toBeNull();
    await act(async () => container.querySelector('button')!.click());
    expect(container.querySelector('input')).toBeNull();
  });

  it('remembers each pane independently through remounts', async () => {
    reduced = true;
    await act(async () => root.render(<Preferences key="first" pane="general" />));
    await act(async () => container.querySelector('button')!.click());
    await act(async () => root.render(<Preferences key="second" pane="recording" />));
    expect(container.querySelector('input')).toBeNull();
    await act(async () => root.render(<Preferences key="third" pane="general" />));
    expect(container.querySelector('input')?.value).toBe('saved');
    expect(JSON.parse(localStorage.getItem(SETTINGS_ADVANCED_KEY)!)).toEqual({ version: 1, panes: { general: true } });
  });

  it.each(['{', JSON.stringify({ version: 2, panes: { general: true } }), 'x'.repeat(2049), JSON.stringify({ version: 1, panes: [] })])('fails closed for invalid UI state %s', raw => {
    localStorage.setItem(SETTINGS_ADVANCED_KEY, raw);
    expect(readSettingsAdvanced()).toEqual({});
  });

  it('bounds known pane keys and ignores non-boolean or sensitive values', () => {
    localStorage.setItem(SETTINGS_ADVANCED_KEY, JSON.stringify({ version: 1, panes: { general: true, recording: 'yes', '/private/path': true, token: 'secret' } }));
    expect(readSettingsAdvanced()).toEqual({ general: true });
  });
});
