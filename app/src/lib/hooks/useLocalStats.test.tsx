import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { InsightsView } from '../../components/home/InsightsView';
import { useLocalStats } from './useLocalStats';
import { loadStats, resetStats, updateStats } from '../stats';
import { IDLE_MEETING_STATUS } from '../meetings';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  emitTo: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: () => false }));
vi.mock('@tauri-apps/api/event', () => ({
  emitTo: mocks.emitTo,
  listen: vi.fn(async (name: string, listener: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(name, listener);
    return () => mocks.listeners.delete(name);
  }),
}));

function emit(name: string, payload: unknown) {
  const listener = mocks.listeners.get(name);
  if (!listener) throw new Error(`Missing listener: ${name}`);
  listener({ payload });
}



function Harness() {
  const statsVersion = useLocalStats();
  return <InsightsView statsVersion={statsVersion} onBackToHome={() => {}} />;
}

describe('main local stats completion ownership', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(async () => {
    localStorage.clear();
    mocks.listeners.clear();
    mocks.invoke.mockReset();
    mocks.emitTo.mockReset();
    mocks.emitTo.mockImplementation(async (target: string, event: string, receipt: unknown) => {
      expect(target).toBe('main');
      emit(event, receipt);
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it('uses frozen native capture duration and successful terminal generation, never the display timer', async () => {
    await act(async () => {
      emit('meeting-status-changed', IDLE_MEETING_STATUS);
      emit('meeting-status-changed', { ...IDLE_MEETING_STATUS, generation: 1, sessionId: 'private-session', phase: 'recording', elapsedMs: 30_000 });
      emit('meeting-status-changed', { ...IDLE_MEETING_STATUS, generation: 1, sessionId: 'private-session', phase: 'processing', elapsedMs: 90_000 });
      emit('meeting-status-changed', { ...IDLE_MEETING_STATUS, generation: 1 });
      emit('meeting-status-changed', { ...IDLE_MEETING_STATUS, generation: 1 });
      emit('meeting-status-changed', { ...IDLE_MEETING_STATUS, generation: 2, sessionId: 'failed-session', phase: 'processing', elapsedMs: 99_000 });
      emit('meeting-status-changed', { ...IDLE_MEETING_STATUS, generation: 2, phase: 'failed' });
      emit('meeting-status-changed', { ...IDLE_MEETING_STATUS, generation: 1 });
    });
    expect(loadStats().activity.meetings).toEqual({ count: 1, totalMinutes: 1.5, summariesGenerated: 0 });
    expect(container.textContent).toContain('1.5 minutes');
    expect(JSON.stringify(loadStats())).not.toContain('private-session');
  });

  it('counts Paste Last only at the shared success event for UI, tray, and shortcut', async () => {
    await act(async () => {
      for (const kind of ['auto_pasted', 'clipboard_only', 'auto_pasted', 'failed', 'empty', 'busy']) {
        emit('delivery-retry-feedback', { kind, message: 'PRIVATE_MESSAGE' });
      }
    });
    expect(loadStats().activity.pasteLastUses).toBe(3);
    expect(JSON.stringify(loadStats())).not.toContain('PRIVATE_MESSAGE');
  });

  it('updates the mounted Insights when dictation finishes and when counters reset', async () => {
    await act(async () => {
      updateStats('one two three four', 2);
      emit('delivery-retry-feedback', { kind: 'auto_pasted' });
    });
    expect(container.querySelector('[data-stats="headline"]')?.textContent).toContain('Last dictation');
    expect(loadStats().totalRecordings).toBe(1);
    await act(async () => resetStats());
    await act(async () => {
    });
    expect(loadStats().activity.transforms.runs).toBe(0);
    expect(loadStats().activity.transforms.approved).toBe(0);
    expect(loadStats().activity.pasteLastUses).toBe(0);
    expect(container.querySelector('[data-stats="headline"]')?.textContent).toContain('No dictations yet');
  });

  it('drops listeners on unmount and does not count restored historical statuses', async () => {
    await act(async () => root.unmount());
    expect(mocks.listeners.size).toBe(0);
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
    expect(loadStats().activity.meetings.count).toBe(0);
    expect(loadStats().activity.meetings.summariesGenerated).toBe(0);
  });
});
