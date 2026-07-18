import { act, useRef, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { VocabScanSummary } from '../../lib/settings';
import { VocabTermsModal } from './VocabTermsModal';

const SUMMARY: VocabScanSummary = {
  files: 2,
  skipped: 0,
  terms: 2,
  bytes: 24,
  capped: false,
  ms: 4,
  sampleTerms: ['useEffect', 'scanId'],
  rankedTerms: [
    { term: 'useEffect', freq: 4 },
    { term: 'scanId', freq: 2 },
  ],
  whisperCount: 2,
  adopted: true,
};

describe('VocabTermsModal accessibility', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(async () => {
    vi.useFakeTimers();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);

    function Harness() {
      const [open, setOpen] = useState(false);
      const openerRef = useRef<HTMLButtonElement>(null);
      return (
        <>
          <button ref={openerRef} type="button" onClick={() => setOpen(true)}>
            View all terms
          </button>
          {open && (
            <VocabTermsModal
              summary={SUMMARY}
              folder="/project"
              onClose={() => setOpen(false)}
              returnFocusRef={openerRef}
            />
          )}
        </>
      );
    }

    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it('labels the dialog, traps Tab at both edges, and restores opener focus', async () => {
    const opener = container.querySelector('button') as HTMLButtonElement;
    await act(async () => opener.click());
    await act(async () => vi.advanceTimersByTime(60));

    const dialog = container.querySelector('[role="dialog"]') as HTMLDivElement;
    expect(dialog).not.toBeNull();
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    const titleId = dialog.getAttribute('aria-labelledby');
    expect(titleId).toBeTruthy();
    expect(document.getElementById(titleId!)?.textContent).toContain('Scanned vocabulary');

    const search = dialog.querySelector('[aria-label="Filter scanned terms"]');
    expect(document.activeElement).toBe(search);

    const close = dialog.querySelector('[aria-label="Close"]') as HTMLButtonElement;
    const copy = Array.from(dialog.querySelectorAll('button')).find((button) =>
      button.textContent?.includes('Copy all'),
    ) as HTMLButtonElement;

    opener.focus();
    await act(async () => {
      document.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }),
      );
    });
    expect(document.activeElement).toBe(close);

    await act(async () => {
      document.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true, cancelable: true }),
      );
    });
    expect(document.activeElement).toBe(copy);

    copy.focus();
    await act(async () => {
      document.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }),
      );
    });
    expect(document.activeElement).toBe(close);

    await act(async () => {
      document.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }),
      );
    });
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(opener);
  });
});
