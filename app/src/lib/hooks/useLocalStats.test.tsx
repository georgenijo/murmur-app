import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { InsightsView } from '../../components/home/InsightsView';
import { useLocalStats } from './useLocalStats';
import { loadStats, resetStats, updateStats } from '../stats';
import { LOCAL_STATS_COMPLETION } from '../localStatsEvents';
import { IDLE_MEETING_STATUS } from '../meetings';
import { confirmLearnedCorrection, proposeLearnedCorrection, proposeSpecificLearnedCorrection } from '../correctAndTeach';

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

  it('routes both correction proposal commands and taught scope through main without content', async () => {
    mocks.invoke.mockResolvedValueOnce({ kind: 'unsafe', reason: 'no safe diff' });
    await act(async () => { await proposeLearnedCorrection('secret transcript', 'secret edit'); });
    mocks.invoke.mockResolvedValueOnce({ kind: 'proposal', proposalId: 21, originalText: 'PRIVATE_TEXT' });
    await act(async () => { await proposeLearnedCorrection('secret transcript', 'secret edit'); });
    mocks.invoke.mockResolvedValueOnce({ kind: 'proposal', proposalId: 22, source: 'PRIVATE_TERM' });
    await act(async () => { await proposeSpecificLearnedCorrection('secret transcript', 'secret', 'private'); });
    for (const [proposalId, scope] of [
      [21, { kind: 'global' }],
      [22, { kind: 'app', bundleId: 'PRIVATE_APP' }],
      [23, { kind: 'project', bundleId: 'PRIVATE_APP', root: 'PRIVATE_ROOT' }],
    ] as const) {
      mocks.invoke.mockResolvedValueOnce({ id: 'entry', payload: { source: 'PRIVATE_RULE' } });
      await act(async () => { await confirmLearnedCorrection(proposalId, scope); });
    }
    await act(async () => emit(LOCAL_STATS_COMPLETION, { kind: 'correction_taught', proposalId: 22, scope: 'app' }));
    expect(loadStats().activity.corrections).toEqual({ proposed: 2, taught: 3, byScope: { global: 1, app: 1, project: 1 } });
    expect(mocks.emitTo.mock.calls).toEqual([
      ['main', LOCAL_STATS_COMPLETION, { kind: 'correction_proposed', proposalId: 21 }],
      ['main', LOCAL_STATS_COMPLETION, { kind: 'correction_proposed', proposalId: 22 }],
      ['main', LOCAL_STATS_COMPLETION, { kind: 'correction_taught', proposalId: 21, scope: 'global' }],
      ['main', LOCAL_STATS_COMPLETION, { kind: 'correction_taught', proposalId: 22, scope: 'app' }],
      ['main', LOCAL_STATS_COMPLETION, { kind: 'correction_taught', proposalId: 23, scope: 'project' }],
    ]);
    expect(JSON.stringify(loadStats())).not.toContain('PRIVATE');
    expect(container.textContent).toContain('Corrections taught');
  });

  it('does not turn a successful correction into a failure when stats delivery fails', async () => {
    mocks.emitTo.mockRejectedValueOnce(new Error('stats unavailable'));
    const entry = { id: 'confirmed' };
    mocks.invoke.mockResolvedValueOnce(entry);
    await expect(confirmLearnedCorrection(1, { kind: 'global' })).resolves.toEqual(entry);
    mocks.invoke.mockRejectedValueOnce(new Error('confirmation failed'));
    await expect(confirmLearnedCorrection(2, { kind: 'global' })).rejects.toThrow('confirmation failed');
    expect(mocks.emitTo).toHaveBeenCalledTimes(1);
  });

  it('updates the mounted Insights when dictation finishes and when counters reset', async () => {
    await act(async () => {
      updateStats('one two three four', 2, 'builtin.technical');
      emit('delivery-retry-feedback', { kind: 'auto_pasted' });
    });
    expect(container.querySelector('[data-stats="headline"]')?.textContent).toContain('Last dictation');
    expect(loadStats().activity.recordingsByMode).toEqual({ 'builtin.technical': 1 });
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
