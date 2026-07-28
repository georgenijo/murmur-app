import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest';
import { WhatsNewModal } from './WhatsNewModal';

describe('WhatsNewModal', () => {
  let container: HTMLDivElement;
  let root: Root;
  let onDismiss: Mock<() => void>;

  beforeEach(() => {
    vi.useFakeTimers();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    onDismiss = vi.fn();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it('renders sanitized release notes for the installed version and focuses the action', async () => {
    await act(async () => {
      root.render(
        <WhatsNewModal
          update={{
            version: '0.22.0',
            notes: '## New Features\n\n- Faster transcription\n\n<script>bad()</script>',
          }}
          onDismiss={onDismiss}
        />,
      );
      vi.advanceTimersByTime(60);
    });

    const dialog = container.querySelector('[role="dialog"]') as HTMLDivElement;
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.textContent).toContain("What's new in Murmur 0.22.0");
    expect(dialog.textContent).toContain('Faster transcription');
    expect(dialog.querySelector('script')).toBeNull();
    expect(document.activeElement?.textContent).toContain('Start using Murmur');

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
    });
    expect(document.activeElement?.textContent).toContain('Start using Murmur');
  });

  it('dismisses from the primary action and Escape', async () => {
    await act(async () => {
      root.render(
        <WhatsNewModal
          update={{ version: '0.22.0', notes: 'Bug fixes.' }}
          onDismiss={onDismiss}
        />,
      );
    });

    const button = container.querySelector('button') as HTMLButtonElement;
    await act(async () => button.click());
    expect(onDismiss).toHaveBeenCalledTimes(1);

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(onDismiss).toHaveBeenCalledTimes(2);
  });
});
