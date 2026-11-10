import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  listen: vi.fn(async (name: string, listener: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(name, listener);
    return () => mocks.listeners.delete(name);
  }),
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

import { initializeDiscoveryAfterSetup, loadDiscoveryState } from '../discovery';
import { addHistoryEntry, type HistoryEntry } from '../history';
import { useDiscovery } from './useDiscovery';


describe('discovery event receipts', () => {
  let container: HTMLDivElement;
  let root: Root;
  let entries: HistoryEntry[];

  function Harness() {
    const discovery = useDiscovery({ enabled: true, historyEntries: entries });
    return <output>{discovery.state?.pendingHints.join(',')}</output>;
  }

  beforeEach(async () => {
    localStorage.clear();
    mocks.listeners.clear();
    initializeDiscoveryAfterSetup();
    entries = [];
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('queues each content-free first-use hint once and restores the queue after remount', async () => {
    const emit = (name: string, payload: unknown) => {
      const listener = mocks.listeners.get(name);
      if (!listener) throw new Error(`Missing listener: ${name}`);
      listener({ payload });
    };
    await act(async () => {
      emit('hotkey-tap-rejected', { reason: 'second_tap_expired', privateText: 'SECRET' });
      emit('hotkey-tap-rejected', { reason: 'second_tap_expired' });
      emit('transcription-complete', {
        text: 'SECRET TRANSCRIPT',
        teachingContext: { appBundleId: 'com.apple.Safari', appLabel: 'SECRET WINDOW' },
      });
      entries = [{ id: 'SECRET ENTRY', text: 'SECRET HISTORY', timestamp: 1, duration: 1 }];
      root.render(<Harness />);
    });

    expect(loadDiscoveryState()?.pendingHints).toEqual([
      'double_tap_timing', 'browser_modes', 'correct_and_teach',
    ]);
    expect(localStorage.getItem('murmur_discovery_v2')).not.toContain('SECRET');

    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
    expect(loadDiscoveryState()?.pendingHints).toHaveLength(3);
  });

  it('detects a new entry by identity when retained history stays at its cap', async () => {
    await act(async () => root.unmount());
    initializeDiscoveryAfterSetup();
    entries = Array.from({ length: 200 }, (_, index) => ({
      id: `entry-${index}`,
      text: 'fixture',
      timestamp: index,
      duration: 1,
    }));
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
    expect(loadDiscoveryState()?.pendingHints).toEqual([]);
    entries = addHistoryEntry(entries, 'new fixture', 1);
    expect(entries).toHaveLength(200);
    await act(async () => root.render(<Harness />));
    expect(loadDiscoveryState()?.pendingHints).toContain('correct_and_teach');
  });
});
