import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AboutModal } from './AboutModal';

vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '0.58.0') }));

function Harness() {
  const [open, setOpen] = useState(false);
  return <><button onClick={() => setOpen(true)}>About</button><AboutModal isOpen={open} onClose={() => setOpen(false)} /></>;
}

describe('AboutModal lifecycle', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('enters a named dialog, contains both Tab directions, dismisses and restores its invoker', async () => {
    await act(async () => root.render(<Harness />));
    const opener = container.querySelector('button')!;
    opener.focus();
    await act(async () => opener.click());
    const dialog = container.querySelector('[role="dialog"]')!;
    const close = dialog.querySelector('button')!;
    expect(dialog.getAttribute('aria-label')).toBe('About Murmur');
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(document.activeElement).toBe(close);
    for (const shiftKey of [false, true]) {
      await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey, bubbles: true })));
      expect(document.activeElement).toBe(close);
    }
    expect(dialog.textContent).toContain('Private dictation and meeting transcription.');
    expect(dialog.textContent).toContain('Audio and transcripts stay on this Mac');
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(opener);
    await act(async () => opener.click());
    await act(async () => container.querySelector('[role="dialog"] button')!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(document.activeElement).toBe(opener);
  });
});
