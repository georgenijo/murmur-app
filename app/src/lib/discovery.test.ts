import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DISCOVERY_STORAGE_KEY, initializeDiscoveryAfterSetup, initializeGrandfatheredDiscovery,
  loadDiscoveryState, enqueueDiscoveryHint, dismissDiscoveryHint, resetDiscovery,
} from './discovery';

describe('contextual hint persistence', () => {
  beforeEach(() => localStorage.clear());

  it('migrates only validated hint IDs and removes obsolete local progress', () => {
    localStorage.setItem('murmur_discovery_v1', JSON.stringify({
      version: 1, completed: ['meeting'], baseline: { meetings: 2 },
      pendingHints: ['browser_modes', 'correct_and_teach'], dismissedHints: ['browser_modes'],
    }));
    expect(initializeGrandfatheredDiscovery()).toEqual({
      version: 2, pendingHints: ['correct_and_teach'], dismissedHints: ['browser_modes'],
    });
    expect(localStorage.getItem('murmur_discovery_v1')).toBeNull();
    expect(JSON.parse(localStorage.getItem(DISCOVERY_STORAGE_KEY)!)).toEqual(loadDiscoveryState());
    expect(localStorage.getItem(DISCOVERY_STORAGE_KEY)).not.toContain('completed');
    expect(localStorage.getItem(DISCOVERY_STORAGE_KEY)).not.toContain('baseline');
  });

  it('queues each hint once and preserves dismissal across initialization', () => {
    initializeDiscoveryAfterSetup();
    enqueueDiscoveryHint('browser_modes');
    enqueueDiscoveryHint('browser_modes');
    expect(loadDiscoveryState()?.pendingHints).toEqual(['browser_modes']);
    dismissDiscoveryHint('browser_modes');
    enqueueDiscoveryHint('browser_modes');
    expect(initializeGrandfatheredDiscovery()?.pendingHints).toEqual([]);
    expect(loadDiscoveryState()?.dismissedHints).toEqual(['browser_modes']);
  });

  it('rejects malformed legacy hints without preserving retired data', () => {
    localStorage.setItem('murmur_discovery_v1', JSON.stringify({ version: 1, pendingHints: ['unknown'], dismissedHints: [] }));
    expect(initializeGrandfatheredDiscovery()).toEqual({ version: 2, pendingHints: [], dismissedHints: [] });
    expect(localStorage.getItem('murmur_discovery_v1')).toBeNull();
  });

  it('resets both storage versions on setup rerun', () => {
    initializeDiscoveryAfterSetup();
    localStorage.setItem('murmur_discovery_v1', '{}');
    resetDiscovery();
    expect(localStorage.getItem(DISCOVERY_STORAGE_KEY)).toBeNull();
    expect(localStorage.getItem('murmur_discovery_v1')).toBeNull();
  });
});

describe('hint migration recovery', () => {
  const legacyKey = 'murmur_discovery_v1';
  const stored = (version = 1) => ({ version, pendingHints: ['double_tap_timing'], dismissedHints: ['browser_modes'] });
  beforeEach(() => localStorage.clear());

  it.each([1, 2])('keeps individually valid IDs in v%i arrays with unknown, duplicate and non-string values', (version) => {
    localStorage.setItem(version === 1 ? legacyKey : DISCOVERY_STORAGE_KEY, JSON.stringify({
      version, pendingHints: ['unknown', 'browser_modes', 'double_tap_timing', null, 'double_tap_timing'],
      dismissedHints: ['browser_modes', 1, 'future_hint', 'browser_modes'],
    }));
    expect(loadDiscoveryState()).toEqual(stored(2));
    expect(loadDiscoveryState()).toEqual(stored(2));
    expect(localStorage.getItem(legacyKey)).toBeNull();
  });

  it.each(['not JSON', JSON.stringify({ version: 2, pendingHints: 'invalid', dismissedHints: [] })])('falls back from an invalid current record to valid legacy hints', (invalid) => {
    localStorage.setItem(DISCOVERY_STORAGE_KEY, invalid);
    localStorage.setItem(legacyKey, JSON.stringify(stored()));
    expect(loadDiscoveryState()).toEqual(stored(2));
    expect(JSON.parse(localStorage.getItem(DISCOVERY_STORAGE_KEY)!)).toEqual(stored(2));
    expect(localStorage.getItem(legacyKey)).toBeNull();
  });

  it('recovers an unreadable current record without legacy data during initialization', () => {
    localStorage.setItem(DISCOVERY_STORAGE_KEY, 'not JSON');
    expect(loadDiscoveryState()).toBeNull();
    expect(initializeGrandfatheredDiscovery()).toEqual({ version: 2, pendingHints: [], dismissedHints: [] });
    expect(loadDiscoveryState()).toEqual({ version: 2, pendingHints: [], dismissedHints: [] });
  });

  it('removes unparseable legacy JSON even alongside a valid current record', () => {
    localStorage.setItem(legacyKey, 'not JSON');
    expect(loadDiscoveryState()).toBeNull();
    expect(localStorage.getItem(legacyKey)).toBeNull();
    localStorage.setItem(legacyKey, 'not JSON');
    localStorage.setItem(DISCOVERY_STORAGE_KEY, JSON.stringify(stored(2)));
    expect(loadDiscoveryState()).toEqual(stored(2));
    expect(localStorage.getItem(legacyKey)).toBeNull();
  });

  it('preserves dismissals from both keys and migrates only once', () => {
    localStorage.setItem(legacyKey, JSON.stringify(stored()));
    localStorage.setItem(DISCOVERY_STORAGE_KEY, JSON.stringify({ version: 2,
      pendingHints: ['browser_modes', 'correct_and_teach'], dismissedHints: ['double_tap_timing'] }));
    const expected = { version: 2, pendingHints: ['correct_and_teach'], dismissedHints: ['double_tap_timing', 'browser_modes'] };
    expect(loadDiscoveryState()).toEqual(expected);
    expect(loadDiscoveryState()).toEqual(expected);
    expect(localStorage.getItem(legacyKey)).toBeNull();
  });

  it.each([false, true])('ignores retired progress fields (dismissed: %s)', (dismissed) => {
    localStorage.setItem(legacyKey, JSON.stringify({ ...stored(), checklistDismissed: dismissed,
      completed: ['meeting', 'mode_binding'], baseline: { meetings: 4, modeBindings: ['fixture'] } }));
    expect(loadDiscoveryState()).toEqual(stored(2));
    expect(Object.keys(JSON.parse(localStorage.getItem(DISCOVERY_STORAGE_KEY)!))).toEqual(['version', 'pendingHints', 'dismissedHints']);
  });

  it('preserves recoverable legacy data when replacement storage fails', () => {
    localStorage.setItem(legacyKey, JSON.stringify(stored()));
    const write = vi.spyOn(localStorage, 'setItem').mockImplementation(() => { throw new Error('quota'); });
    expect(loadDiscoveryState()).toEqual(stored(2));
    expect(localStorage.getItem(legacyKey)).not.toBeNull();
    write.mockRestore();
    expect(loadDiscoveryState()).toEqual(stored(2));
    expect(localStorage.getItem(legacyKey)).toBeNull();
  });
});

describe('hint storage failure isolation', () => {
  const legacyKey = 'murmur_discovery_v1';
  const empty = { version: 2, pendingHints: [], dismissedHints: [] };
  beforeEach(() => localStorage.clear());

  it.each([DISCOVERY_STORAGE_KEY, legacyKey])('keeps readable dismissals when pending data is malformed in %s', (key) => {
    localStorage.setItem(key, JSON.stringify({ version: key === legacyKey ? 1 : 2,
      pendingHints: 'invalid', dismissedHints: ['browser_modes', null, 'unknown'] }));
    localStorage.setItem(key === legacyKey ? DISCOVERY_STORAGE_KEY : legacyKey, JSON.stringify({
      version: key === legacyKey ? 2 : 1, pendingHints: ['browser_modes', 'correct_and_teach'], dismissedHints: 'invalid',
    }));
    const expected = { version: 2, pendingHints: ['correct_and_teach'], dismissedHints: ['browser_modes'] };
    expect(initializeGrandfatheredDiscovery()).toEqual(expected);
    expect(loadDiscoveryState()).toEqual(expected);
    expect(localStorage.getItem(legacyKey)).toBeNull();
  });

  it.each([DISCOVERY_STORAGE_KEY, legacyKey])('does not overwrite either key when reading %s throws', (failedKey) => {
    const current = JSON.stringify({ ...empty, dismissedHints: ['browser_modes'] });
    const legacy = JSON.stringify({ version: 1, pendingHints: ['browser_modes'], dismissedHints: ['correct_and_teach'] });
    localStorage.setItem(DISCOVERY_STORAGE_KEY, current);
    localStorage.setItem(legacyKey, legacy);
    const originalRead = localStorage.getItem.bind(localStorage);
    const read = vi.spyOn(localStorage, 'getItem').mockImplementation((key) => {
      if (key === failedKey) throw new Error('read unavailable');
      return originalRead(key);
    });
    const write = vi.spyOn(localStorage, 'setItem');
    const remove = vi.spyOn(localStorage, 'removeItem');
    const recovered = initializeGrandfatheredDiscovery();
    expect(recovered.dismissedHints).toEqual(failedKey === legacyKey ? ['browser_modes'] : ['correct_and_teach']);
    enqueueDiscoveryHint('double_tap_timing');
    dismissDiscoveryHint('double_tap_timing');
    expect(write).not.toHaveBeenCalled();
    expect(remove).not.toHaveBeenCalled();
    expect(originalRead(DISCOVERY_STORAGE_KEY)).toBe(current);
    expect(originalRead(legacyKey)).toBe(legacy);
    read.mockRestore(); write.mockRestore(); remove.mockRestore();
    expect(initializeGrandfatheredDiscovery()).toEqual({ ...empty, dismissedHints: ['browser_modes', 'correct_and_teach'] });
    expect(enqueueDiscoveryHint('browser_modes')?.pendingHints).toEqual([]);
  });

  it.each([DISCOVERY_STORAGE_KEY, legacyKey])('uses defaults without writing when %s is unavailable and the other key is absent', (failedKey) => {
    const originalRead = localStorage.getItem.bind(localStorage);
    const read = vi.spyOn(localStorage, 'getItem').mockImplementation((key) => {
      if (key === failedKey) throw new Error('read unavailable');
      return originalRead(key);
    });
    const write = vi.spyOn(localStorage, 'setItem');
    expect(initializeGrandfatheredDiscovery()).toEqual(empty);
    expect(write).not.toHaveBeenCalled();
    read.mockRestore(); write.mockRestore();
    expect(initializeGrandfatheredDiscovery()).toEqual(empty);
    expect(localStorage.getItem(DISCOVERY_STORAGE_KEY)).toBe(JSON.stringify(empty));
  });

  it.each(['not JSON', JSON.stringify({ version: 1, pendingHints: [], dismissedHints: ['correct_and_teach'] })])(
    'retains readable state when legacy cleanup throws (%s)', (legacy) => {
      localStorage.setItem(DISCOVERY_STORAGE_KEY, JSON.stringify({ ...empty, dismissedHints: ['browser_modes'] }));
      localStorage.setItem(legacyKey, legacy);
      const remove = vi.spyOn(localStorage, 'removeItem').mockImplementation(() => { throw new Error('cleanup unavailable'); });
      const expected = { ...empty, dismissedHints: legacy === 'not JSON' ? ['browser_modes'] : ['browser_modes', 'correct_and_teach'] };
      expect(initializeGrandfatheredDiscovery()).toEqual(expected);
      expect(loadDiscoveryState()).toEqual(expected);
      expect(localStorage.getItem(legacyKey)).toBe(legacy);
      remove.mockRestore();
      expect(initializeGrandfatheredDiscovery()).toEqual(expected);
      expect(enqueueDiscoveryHint('browser_modes')?.pendingHints).toEqual([]);
      expect(localStorage.getItem(legacyKey)).toBeNull();
    },
  );

  it('does not persist defaults if cleaning an unreadable legacy record fails', () => {
    localStorage.setItem(legacyKey, 'not JSON');
    const remove = vi.spyOn(localStorage, 'removeItem').mockImplementation(() => { throw new Error('cleanup unavailable'); });
    const write = vi.spyOn(localStorage, 'setItem');
    expect(initializeGrandfatheredDiscovery()).toEqual(empty);
    expect(write).not.toHaveBeenCalled();
    remove.mockRestore(); write.mockRestore();
    expect(initializeGrandfatheredDiscovery()).toEqual(empty);
    expect(localStorage.getItem(legacyKey)).toBeNull();
  });
});
