import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest';
import { CommandPalette } from './CommandPalette';
import type { PaletteCommand } from '../lib/commandPalette';

describe('CommandPalette', () => {
  let container: HTMLDivElement;
  let root: Root;
  let onClose: Mock<() => void>;
  let runs: Record<string, Mock<() => void>>;
  let commands: PaletteCommand[];

  const rows = () => Array.from(container.querySelectorAll('[role="option"]'));
  const selectedRow = () => container.querySelector('[aria-selected="true"]');
  const input = () => container.querySelector('input') as HTMLInputElement;

  async function render(isOpen = true) {
    await act(async () => {
      root.render(<CommandPalette isOpen={isOpen} onClose={onClose} commands={commands} />);
    });
  }

  async function type(value: string) {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(input(), value);
      input().dispatchEvent(new Event('input', { bubbles: true }));
    });
  }

  async function press(key: string) {
    await act(async () => {
      input().dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }));
    });
  }

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    onClose = vi.fn();
    runs = {
      record: vi.fn(),
      logs: vi.fn(),
      delivery: vi.fn(),
    };
    commands = [
      { id: 'record', title: 'Start recording', section: 'Recording', run: runs.record },
      { id: 'logs', title: 'Open log viewer', section: 'Diagnostics', keywords: ['events'], run: runs.logs },
      { id: 'delivery', title: 'Settings: Delivery', section: 'Settings', run: runs.delivery },
    ];
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => { cb(0); return 1; });
    vi.stubGlobal('cancelAnimationFrame', () => {});
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('renders nothing while closed', async () => {
    await render(false);
    expect(container.innerHTML).toBe('');
  });

  it('lists every command and selects the first row', async () => {
    await render();
    expect(rows()).toHaveLength(3);
    expect(selectedRow()?.textContent).toContain('Start recording');
    expect(document.activeElement).toBe(input());
  });

  it('filters as you type', async () => {
    await render();
    await type('log');
    expect(rows()).toHaveLength(1);
    expect(rows()[0].textContent).toContain('Open log viewer');
  });

  it('finds a command by keyword', async () => {
    await render();
    await type('events');
    expect(rows()).toHaveLength(1);
    expect(rows()[0].textContent).toContain('Open log viewer');
  });

  it('shows an empty state when nothing matches', async () => {
    await render();
    await type('zzzz');
    expect(rows()).toHaveLength(0);
    expect(container.textContent).toContain('No matching command');
  });

  it('moves the selection with the arrow keys and wraps', async () => {
    await render();
    await press('ArrowDown');
    expect(selectedRow()?.textContent).toContain('Open log viewer');
    await press('ArrowUp');
    await press('ArrowUp');
    expect(selectedRow()?.textContent).toContain('Settings: Delivery');
  });

  it('runs the selected command on Enter and closes', async () => {
    await render();
    await press('ArrowDown');
    await press('Enter');
    expect(runs.logs).toHaveBeenCalledOnce();
    expect(runs.record).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('does nothing on Enter with no results', async () => {
    await render();
    await type('zzzz');
    await press('Enter');
    expect(onClose).not.toHaveBeenCalled();
    expect(Object.values(runs).every((run) => run.mock.calls.length === 0)).toBe(true);
  });

  it('keeps focus inside the dialog on Tab', async () => {
    await render();
    await press('Tab');
    expect(document.activeElement).toBe(input());
    expect(onClose).not.toHaveBeenCalled();
  });

  it('closes on Escape without running anything', async () => {
    await render();
    await press('Escape');
    expect(onClose).toHaveBeenCalledOnce();
    expect(Object.values(runs).every((run) => run.mock.calls.length === 0)).toBe(true);
  });

  it('runs a command on click', async () => {
    await render();
    await act(async () => {
      (rows()[2] as HTMLButtonElement).click();
    });
    expect(runs.delivery).toHaveBeenCalledOnce();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('closes when the backdrop is clicked but not the dialog', async () => {
    await render();
    const dialog = container.querySelector('[role="dialog"]') as HTMLElement;
    await act(async () => {
      dialog.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    });
    expect(onClose).not.toHaveBeenCalled();
    const backdrop = container.firstElementChild as HTMLElement;
    await act(async () => {
      backdrop.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('resets the query and selection each time it opens', async () => {
    await render();
    await type('log');
    await render(false);
    await render(true);
    expect(input().value).toBe('');
    expect(rows()).toHaveLength(3);
  });

  it('restores focus to the invoking control when dismissed', async () => {
    const trigger = document.createElement('button');
    document.body.appendChild(trigger);
    trigger.focus();
    await render();
    expect(document.activeElement).toBe(input());
    await render(false);
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });

  it('does not steal focus back after a command moves it', async () => {
    const trigger = document.createElement('button');
    const destination = document.createElement('button');
    document.body.append(trigger, destination);
    trigger.focus();
    runs.record.mockImplementation(() => destination.focus());
    await render();
    await press('Enter');
    await render(false);
    expect(document.activeElement).toBe(destination);
    trigger.remove();
    destination.remove();
  });

  it('restores focus after a command that does not move it', async () => {
    const trigger = document.createElement('button');
    document.body.appendChild(trigger);
    trigger.focus();
    await render();
    await press('Enter');
    await render(false);
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });
});
