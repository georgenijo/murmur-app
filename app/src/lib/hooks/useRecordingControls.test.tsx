import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationStatus } from '../types';
import { START_REFUSED_FLASH_MS, useRecordingControls, type RecordingControls } from './useRecordingControls';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

describe('overlay start refused during cleanup', () => {
  let root: Root;
  let container: HTMLDivElement;
  let controls: RecordingControls;
  const statusRef = { current: 'idle' as DictationStatus };
  const disabledRef = { current: false };
  const expandedRef = { current: false };

  function Harness({ status }: { status: DictationStatus }) {
    controls = useRecordingControls({ status, statusRef, disabledRef, expandedRef });
    return null;
  }
  const render = (status: DictationStatus) => act(async () => {
    statusRef.current = status;
    root.render(<Harness status={status} />);
  });

  beforeEach(() => {
    vi.useFakeTimers();
    invokeMock.mockReset();
    container = document.createElement('div');
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    vi.useRealTimers();
  });

  it('flags the refusal, releases locked mode, and clears after the flash window', async () => {
    invokeMock.mockResolvedValueOnce({ type: 'audio_recovering', state: 'recovering' });
    await render('idle');
    await act(async () => controls.handleDoubleClick());
    expect(controls.startRefused).toBe(true);
    expect(controls.lockedMode).toBe(false);
    await act(async () => { vi.advanceTimersByTime(START_REFUSED_FLASH_MS - 1); });
    expect(controls.startRefused).toBe(true);
    await act(async () => { vi.advanceTimersByTime(1); });
    expect(controls.startRefused).toBe(false);
  });

  it('clears the flag as soon as the app leaves idle', async () => {
    invokeMock.mockResolvedValueOnce({ type: 'audio_recovering', state: 'recovering' });
    await render('idle');
    await act(async () => controls.handleDoubleClick());
    expect(controls.startRefused).toBe(true);
    await render('starting');
    expect(controls.startRefused).toBe(false);
  });

  it('does not flag other refusals or a successful start', async () => {
    invokeMock.mockResolvedValueOnce({ type: 'recording_starting', state: 'starting' });
    await render('idle');
    await act(async () => controls.handleDoubleClick());
    expect(controls.startRefused).toBe(false);
    expect(controls.lockedMode).toBe(true);
  });
});
