import { beforeEach, describe, expect, it } from 'vitest';
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
