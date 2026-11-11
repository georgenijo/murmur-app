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

import { DISCOVERY_STORAGE_KEY, initializeDiscoveryAfterSetup, initializeGrandfatheredDiscovery, loadDiscoveryState, dismissDiscoveryHint, enqueueDiscoveryHint } from '../discovery';
import { addHistoryEntry, type HistoryEntry } from '../history';
import { useDiscovery } from './useDiscovery';


describe('discovery event receipts', () => {
  let container: HTMLDivElement;
  let root: Root;
  let entries: HistoryEntry[];

  function Harness() {
    const discovery = useDiscovery({ enabled: true, historyEntries: entries });
    return <>
      <output>{discovery.state?.pendingHints.join(',')}</output>
      {discovery.state?.pendingHints.map((id) => (
        <button key={id} onClick={() => discovery.dismissHint(id)}>Dismiss {id}</button>
      ))}
    </>;
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
      entries = [{ id: 'SECRET ENTRY', text: 'SECRET HISTORY', timestamp: 1, duration: 1 }];
      root.render(<Harness />);
    });

    expect(loadDiscoveryState()?.pendingHints).toEqual([
      'double_tap_timing', 'correct_and_teach',
    ]);
    expect(localStorage.getItem('murmur_discovery_v2')).not.toContain('SECRET');

    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
    expect(loadDiscoveryState()?.pendingHints).toHaveLength(2);
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

  it('visibly dismisses and queues hints while legacy cleanup keeps failing, including after remount', async () => {
    await act(async () => root.unmount());
    const pending = { version: 2, pendingHints: ['double_tap_timing'], dismissedHints: [] };
    localStorage.setItem(DISCOVERY_STORAGE_KEY, JSON.stringify(pending));
    localStorage.setItem('murmur_discovery_v1', JSON.stringify({ ...pending, version: 1 }));
    const cleanup = vi.spyOn(localStorage, 'removeItem').mockImplementation(() => { throw new Error('cleanup unavailable'); });
    try {
      root = createRoot(container);
      await act(async () => root.render(<Harness />));
      expect(container.querySelector('output')?.textContent).toBe('double_tap_timing');
      await act(async () => container.querySelector('button')!.click());
      expect(container.querySelector('output')?.textContent).toBe('');
      expect(container.querySelector('button')).toBeNull();
      expect(JSON.parse(localStorage.getItem(DISCOVERY_STORAGE_KEY)!)).toEqual({ ...pending, pendingHints: [], dismissedHints: ['double_tap_timing'] });
      await act(async () => {
        entries = [{ id: 'new-fixture', text: 'fixture', timestamp: 1, duration: 1 }];
        root.render(<Harness />);
      });
      expect(container.querySelector('output')?.textContent).toBe('correct_and_teach');
      expect(JSON.parse(localStorage.getItem(DISCOVERY_STORAGE_KEY)!)).toEqual({ version: 2, pendingHints: ['correct_and_teach'], dismissedHints: ['double_tap_timing'] });
      await act(async () => root.unmount());
      root = createRoot(container);
      await act(async () => root.render(<Harness />));
      expect(container.querySelector('output')?.textContent).toBe('correct_and_teach');
      expect(container.textContent).not.toContain('double_tap_timing');
      expect(localStorage.getItem('murmur_discovery_v1')).not.toBeNull();
    } finally { cleanup.mockRestore(); }
  });

  it('ignores a retired hint id left in stored discovery state and drops it on the next write', async () => {
    await act(async () => root.unmount());
    localStorage.setItem(DISCOVERY_STORAGE_KEY, JSON.stringify({
      version: 2, pendingHints: ['browser_modes', 'double_tap_timing'], dismissedHints: ['browser_modes'],
    }));
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
    expect(container.querySelector('output')?.textContent).toBe('double_tap_timing');
    expect(loadDiscoveryState()).toEqual({ version: 2, pendingHints: ['double_tap_timing'], dismissedHints: [] });
    expect(localStorage.getItem(DISCOVERY_STORAGE_KEY)).not.toContain('browser_modes');
  });

});

describe('discovery storage failure matrix', () => {
  it('returns and shows only recoverable state, preserves dismissals, and never writes after a failed read', async () => {
    const legacyKey = 'murmur_discovery_v1';
    const keys = [DISCOVERY_STORAGE_KEY, legacyKey];
    type State = ReturnType<typeof loadDiscoveryState>;
    type Call = 'current read' | 'legacy read' | 'write' | 'cleanup';
    type Fault = { call: Call; occurrence: number | 'always'; cleanupAlways?: boolean } | 'unavailable' | null;
    type Operation = 'load' | 'initialize' | 'dismiss' | 'queue';
    const operations: { name: string; steps: Operation[] }[] = [
      { name: 'initial load/migration', steps: ['load'] },
      { name: 'grandfathered initialization', steps: ['initialize'] },
      { name: 'dismiss pending hint', steps: ['dismiss'] },
      { name: 'queue new hint', steps: ['queue'] },
      { name: 'dismiss then queue', steps: ['dismiss', 'queue'] },
      { name: 'queue then dismiss', steps: ['queue', 'dismiss'] },
    ];
    const failures: string[] = [];
    let cells = 0;

    for (const fixture of [
      { name: DISCOVERY_STORAGE_KEY, dismissalKey: DISCOVERY_STORAGE_KEY },
      { name: legacyKey, dismissalKey: legacyKey },
      { name: 'unreadable legacy', dismissalKey: DISCOVERY_STORAGE_KEY },
    ]) {
      const { dismissalKey } = fixture;
      for (const operation of operations) {
        for (const path of ['function', 'hook'] as const) {
          const label = `${fixture.name} / ${operation.name} / ${path}`;
          const exercise = async (fault: Fault, faultLabel: string) => {
            const context = `${label} / ${faultLabel}`;
            const storage = localStorage;
            storage.clear();
            for (const [index, key] of keys.entries()) {
              if (fixture.name === 'unreadable legacy' && key === legacyKey) {
                storage.setItem(key, 'not JSON');
                continue;
              }
              storage.setItem(key, JSON.stringify({ version: index === 0 ? 2 : 1,
                pendingHints: key === dismissalKey ? ['browser_modes'] : ['browser_modes', 'double_tap_timing'],
                dismissedHints: key === dismissalKey ? ['double_tap_timing'] : [],
              }));
            }
            const read = storage.getItem.bind(storage);
            const write = storage.setItem.bind(storage);
            const cleanup = storage.removeItem.bind(storage);
            const counts: Record<Call, number> = { 'current read': 0, 'legacy read': 0, write: 0, cleanup: 0 };
            let failedReads = 0;
            let readsAtCheckpoint = 0;
            let failedReadInPair = false;
            let readableDismissals = new Set<string>();
            const writesAfterFailedRead: Call[] = [];
            const checkpoints: { returned?: State; shown?: State; raw: (string | null)[]; unreadable: boolean; readableDismissals: string[] }[] = [];
            const shouldFail = (call: Call) => {
              counts[call]++;
              return fault !== null && fault !== 'unavailable'
                && ((fault.call === call && (fault.occurrence === 'always' || fault.occurrence === counts[call]))
                  || (fault.cleanupAlways && call === 'cleanup'));
            };
            const reads = vi.spyOn(storage, 'getItem').mockImplementation((key) => {
              const call = key === DISCOVERY_STORAGE_KEY ? 'current read' : 'legacy read';
              if (call === 'current read') failedReadInPair = false;
              if (shouldFail(call)) {
                failedReadInPair = true;
                failedReads++;
                throw new Error('injected read failure');
              }
              const value = read(key);
              if (value !== null) {
                try { for (const id of JSON.parse(value).dismissedHints) readableDismissals.add(id); } catch { /* Unreadable JSON contributes no known dismissal. */ }
              }
              return value;
            });
            const writes = vi.spyOn(storage, 'setItem').mockImplementation((key, value) => {
              if (failedReadInPair) writesAfterFailedRead.push('write');
              if (shouldFail('write')) throw new Error('injected write failure');
              write(key, value);
            });
            const removals = vi.spyOn(storage, 'removeItem').mockImplementation((key) => {
              if (failedReadInPair) writesAfterFailedRead.push('cleanup');
              if (shouldFail('cleanup')) throw new Error('injected cleanup failure');
              cleanup(key);
            });
            const globals = [...new Set([globalThis, window])];
            const descriptors = globals.map((target) => Object.getOwnPropertyDescriptor(target, 'localStorage')!);
            if (fault === 'unavailable') {
              for (const target of globals) Object.defineProperty(target, 'localStorage', { configurable: true, get() {
                failedReadInPair = true;
                failedReads++;
                throw new Error('storage unavailable');
              } });
            }
            let root: Root | undefined;
            const container = document.createElement('div');
            let entries: HistoryEntry[] = [];
            function Harness() {
              const discovery = useDiscovery({ enabled: true, historyEntries: entries });
              return <>
                <output>{JSON.stringify(discovery.state)}</output>
                <button onClick={() => discovery.dismissHint('correct_and_teach')}>Dismiss</button>
              </>;
            }
            const checkpoint = (returned?: State) => {
              checkpoints.push({ ...(returned !== undefined ? { returned } : {}),
                ...(root ? { shown: JSON.parse(container.querySelector('output')!.textContent!) as State } : {}),
                raw: keys.map(read), unreadable: failedReads !== readsAtCheckpoint,
                readableDismissals: [...readableDismissals],
              });
              readsAtCheckpoint = failedReads;
              readableDismissals = new Set();
            };
            try {
              if (path === 'function') {
                for (const step of operation.steps) {
                  const state = step === 'load' ? loadDiscoveryState()
                    : step === 'initialize' ? initializeGrandfatheredDiscovery()
                      : step === 'dismiss' ? dismissDiscoveryHint('correct_and_teach') : enqueueDiscoveryHint('correct_and_teach');
                  checkpoint(state);
                }
              } else {
                // Initialization precedes mounting the consumer; load is the hook's initializer.
                if (operation.steps[0] === 'initialize') checkpoint(initializeGrandfatheredDiscovery());
                document.body.appendChild(container);
                root = createRoot(container);
                await act(async () => root!.render(<Harness />));
                checkpoint();
                for (const step of operation.steps) {
                  if (step === 'load' || step === 'initialize') continue;
                  await act(async () => {
                    if (step === 'dismiss') container.querySelector('button')!.click();
                    else {
                      entries = [{ id: 'matrix-fixture', text: 'fixture', timestamp: 1, duration: 1 }];
                      root!.render(<Harness />);
                    }
                  });
                  checkpoint();
                }
              }
            } finally {
              if (root) await act(async () => root!.unmount());
              container.remove();
              if (fault === 'unavailable') globals.forEach((target, index) => Object.defineProperty(target, 'localStorage', descriptors[index]));
              reads.mockRestore(); writes.mockRestore(); removals.mockRestore();
            }
            expect(writesAfterFailedRead, context).toEqual([]);
            for (const [index, point] of checkpoints.entries()) {
              // Replay each actual disk snapshot after recovery, without changing the live fault trace.
              storage.clear();
              point.raw.forEach((raw, keyIndex) => { if (raw !== null) write(keys[keyIndex], raw); });
              const recovered = loadDiscoveryState();
              expect(recovered?.dismissedHints, `${context} / checkpoint ${index}`).toContain('double_tap_timing');
              if (fault === 'unavailable') {
                if ('shown' in point) expect(point.shown, context).toBeNull();
                if ('returned' in point) expect(point.returned, context).toEqual(operation.steps[0] === 'initialize'
                  ? { version: 2, pendingHints: [], dismissedHints: [] } : null);
              }
              for (const value of [point.returned, point.shown]) {
                if (value === undefined) continue;
                if (!point.unreadable) expect(value, `${context} / checkpoint ${index}`).toEqual(recovered);
                // Unreadable-state cells retain their safe partial/null/session-default contract.
                for (const id of point.readableDismissals) expect(value?.dismissedHints, context).toContain(id);
              }
            }
            return counts;
          };
          const run = async (fault: Fault, faultLabel: string) => {
            cells++;
            try { return await exercise(fault, faultLabel); } catch (error) {
              failures.push(`${label} / ${faultLabel}: ${String(error)}`);
              return null;
            }
          };
          const counts = await run(null, 'no failure');
          if (!counts) continue;
          for (const call of Object.keys(counts) as Call[]) {
            for (let occurrence = 1; occurrence <= counts[call]; occurrence++) await run({ call, occurrence }, `${call} #${occurrence} once`);
            await run({ call, occurrence: 'always' }, `${call} every call`);
          }
          await run('unavailable', 'storage entirely unavailable');
          // Also cover the reviewer's persistent-cleanup + failed mutation-write combination.
          for (let occurrence = 1; occurrence <= counts.write; occurrence++) {
            await run({ call: 'write', occurrence, cleanupAlways: true }, `cleanup every call + write #${occurrence} once`);
          }
        }
      }
    }
    expect(failures, `${cells} matrix cells`).toEqual([]);
  }, 30000);
});
