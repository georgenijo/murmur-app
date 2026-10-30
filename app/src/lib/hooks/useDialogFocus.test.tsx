import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { useDialogFocus } from './useDialogFocus';

it.each([false, true])('restores the explicit sheet caller only when requested (%s)', async (preferExplicitRestore) => {
  const container = document.createElement('div'); document.body.append(container);
  const previous = document.createElement('input');
  const caller = document.createElement('button');
  container.append(previous, caller);
  previous.focus();
  const modal = document.createElement('div'); container.append(modal);
  const modalRoot = createRoot(modal);
  function Dialog() {
    const ref = useDialogFocus(vi.fn(), () => caller, preferExplicitRestore);
    return <div ref={ref} role="dialog" aria-modal="true"><button>Close sheet</button></div>;
  }
  try {
    await act(async () => modalRoot.render(<Dialog />));
    expect(document.activeElement?.textContent).toBe('Close sheet');
    await act(async () => modalRoot.unmount());
    expect(document.activeElement).toBe(preferExplicitRestore ? caller : previous);
  } finally { container.remove(); }
});

it('ignores warm hidden Settings sheets and resumes containment when their surface returns', async () => {
  const container = document.createElement('div'); document.body.append(container);
  const root = createRoot(container);
  const onCancel = vi.fn();
  function Dialog() {
    const ref = useDialogFocus(onCancel);
    return <div ref={ref} role="dialog" aria-modal="true"><button>Close sheet</button></div>;
  }
  function View({ hidden }: { hidden: boolean }) {
    return <><button id="outside">Home action</button><section aria-hidden={hidden} {...(hidden ? { inert: '' } : {})}><Dialog /></section></>;
  }
  try {
    await act(async () => root.render(<View hidden={false} />));
    expect(document.activeElement?.textContent).toBe('Close sheet');
    await act(async () => root.render(<View hidden />));
    const outside = container.querySelector<HTMLButtonElement>('#outside')!; outside.focus();
    for (const key of ['Tab', 'Escape']) {
      const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
      await act(async () => { outside.dispatchEvent(event); });
      expect(event.defaultPrevented).toBe(false);
    }
    expect(document.activeElement).toBe(outside);
    expect(onCancel).not.toHaveBeenCalled();
    await act(async () => root.render(<View hidden={false} />));
    const event = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    await act(async () => { outside.dispatchEvent(event); });
    expect(event.defaultPrevented).toBe(true);
    expect(document.activeElement?.textContent).toBe('Close sheet');
  } finally { await act(async () => root.unmount()); container.remove(); }
});
