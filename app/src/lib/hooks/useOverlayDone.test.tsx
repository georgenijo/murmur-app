import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationStatus } from '../types';
import { useOverlayDone } from './useOverlayDone';

describe('overlay finish ownership', () => {
  let root: Root; let container: HTMLDivElement;
  function Harness({ status }: { status: DictationStatus }) { return <span>{String(useOverlayDone(status))}</span>; }
  const render = async (status: DictationStatus) => { await act(async () => root.render(<Harness status={status} />)); };
  beforeEach(() => { vi.useFakeTimers(); container = document.createElement('div'); root = createRoot(container); });
  afterEach(async () => { await act(async () => root.unmount()); vi.useRealTimers(); });
  it('holds Done only for 400ms after processing, never after an ordinary stop', async () => {
    await render('recording'); await render('idle'); expect(container.textContent).toBe('false');
    await render('processing'); await render('idle'); expect(container.textContent).toBe('true');
    await act(async () => vi.advanceTimersByTime(399)); expect(container.textContent).toBe('true');
    await act(async () => vi.advanceTimersByTime(1)); expect(container.textContent).toBe('false');
  });
  it('cancels a finish timer on immediate restart and cannot clear a newer finish', async () => {
    await render('processing'); await render('idle');
    await act(async () => vi.advanceTimersByTime(200));
    await render('starting'); expect(container.textContent).toBe('false');
    await render('recording'); await render('processing'); await render('idle');
    await act(async () => vi.advanceTimersByTime(200)); expect(container.textContent).toBe('true');
    await act(async () => vi.advanceTimersByTime(200)); expect(container.textContent).toBe('false');
  });
});
