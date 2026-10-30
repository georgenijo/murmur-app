import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { defaultButtonKeyDown } from './defaultButton';
import { ConfirmActionButton } from '../components/ui/ConfirmActionButton';

let container: HTMLDivElement;
let root: Root;
beforeEach(() => { container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
async function key(element: HTMLElement, value: string, extra: KeyboardEventInit = {}) {
  const event = new KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true, ...extra });
  await act(async () => { element.dispatchEvent(event); });
  return event;
}

describe('Return defaults and destructive confirmations', () => {
  it('routes Return from Cancel or a field to the current enabled recoverable confirmation once', async () => {
    const save = vi.fn(); const cancel = vi.fn();
    const view = (disabled = false) => <div onKeyDown={defaultButtonKeyDown}>
      <div role="dialog" onKeyDown={defaultButtonKeyDown}>
        <input aria-label="Name" /><textarea aria-label="Body" /><select><option>One</option></select>
        <button onClick={cancel}>Cancel</button><button data-default-button disabled={disabled} onClick={save}>Save</button>
      </div></div>;
    await act(async () => root.render(view()));
    const cancelButton = container.querySelector('button')!;
    expect((await key(cancelButton, 'Enter')).defaultPrevented).toBe(true);
    expect(save).toHaveBeenCalledTimes(1); expect(cancel).not.toHaveBeenCalled();
    await key(container.querySelector('input')!, 'Enter'); expect(save).toHaveBeenCalledTimes(2);
    expect((await key(container.querySelector('textarea')!, 'Enter')).defaultPrevented).toBe(false);
    expect((await key(container.querySelector('select')!, 'Enter')).defaultPrevented).toBe(false);
    await key(cancelButton, 'Enter', { repeat: true });
    await key(cancelButton, 'Enter', { isComposing: true });
    expect(save).toHaveBeenCalledTimes(2);
    await act(async () => root.render(view(true)));
    await key(cancelButton, 'Enter'); expect(save).toHaveBeenCalledTimes(2);
  });

  it('never treats a standing action as a default and leaves Space to the focused button', async () => {
    const action = vi.fn();
    await act(async () => root.render(<div onKeyDown={defaultButtonKeyDown}><button onClick={action}>Start</button></div>));
    const button = container.querySelector('button')!;
    expect((await key(button, 'Enter')).defaultPrevented).toBe(true);
    expect((await key(button, ' ')).defaultPrevented).toBe(false);
    expect(action).not.toHaveBeenCalled();
  });

  it('requires two activations, ignores Return while armed, and disarms on blur and Escape', async () => {
    const remove = vi.fn();
    await act(async () => root.render(<ConfirmActionButton label="Remove model" confirmLabel="Confirm remove" onConfirm={remove} />));
    const button = container.querySelector('button')!;
    await act(async () => { button.focus(); button.click(); });
    expect(button.textContent).toBe('Confirm remove…');
    expect((await key(button, 'Enter')).defaultPrevented).toBe(true); expect(remove).not.toHaveBeenCalled();
    await key(button, 'Escape'); expect(button.textContent).toBe('Remove model…');
    await act(async () => { button.click(); button.blur(); }); expect(button.textContent).toBe('Remove model…');
    await act(async () => button.click()); expect(remove).not.toHaveBeenCalled();
    await act(async () => button.click()); expect(remove).toHaveBeenCalledTimes(1);
  });

  it.each(['shiftKey', 'metaKey', 'ctrlKey', 'altKey'] as const)('blocks %s Return on an armed button while preserving field modifiers and IME', async (modifier) => {
    const remove = vi.fn();
    await act(async () => root.render(<div onKeyDown={defaultButtonKeyDown}>
      <input /><textarea /><ConfirmActionButton label="Delete" onConfirm={remove} />
    </div>));
    const button = container.querySelector('button')!;
    await act(async () => button.click());
    expect((await key(button, 'Enter', { [modifier]: true })).defaultPrevented).toBe(true);
    expect((await key(container.querySelector('input')!, 'Enter', { [modifier]: true })).defaultPrevented).toBe(false);
    expect((await key(container.querySelector('textarea')!, 'Enter', { [modifier]: true })).defaultPrevented).toBe(false);
    expect((await key(button, 'Enter', { [modifier]: true, isComposing: true })).defaultPrevented).toBe(false);
    expect(remove).not.toHaveBeenCalled();
  });
});
