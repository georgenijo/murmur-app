import { beforeEach, describe, expect, it } from 'vitest';
import {
  DISCOVERY_STORAGE_KEY,
  completeChecklistItem,
  dismissDiscoveryChecklist,
  dismissDiscoveryHint,
  enqueueDiscoveryHint,
  initializeDiscoveryAfterSetup,
  initializeGrandfatheredDiscovery,
  loadDiscoveryState,
  reopenDiscoveryChecklist,
  resetDiscovery,
  syncDiscoveryEvidence,
  type DiscoveryEvidence,
} from './discovery';
import { isOnboardingComplete, markOnboardingComplete, resetOnboarding } from './onboarding';

const emptyEvidence = (): DiscoveryEvidence => ({
  transforms: 0,
  meetings: 0,
  correctionsTaught: 0,
  queriesRun: 0,
  modeBindings: [],
  voiceQueryConfigured: false,
});

describe('post-setup discovery state', () => {
  beforeEach(() => localStorage.clear());

  it('starts a fresh setup cycle with every row incomplete, then follows live evidence', () => {
    initializeDiscoveryAfterSetup({
      ...emptyEvidence(),
      transforms: 8,
      meetings: 3,
      correctionsTaught: 2,
      queriesRun: 5,
      modeBindings: ['com.example.Mail:builtin.email'],
      voiceQueryConfigured: true,
    });
    expect(loadDiscoveryState()?.completed).toEqual([]);

    syncDiscoveryEvidence({
      ...emptyEvidence(),
      transforms: 9,
      meetings: 4,
      correctionsTaught: 3,
      queriesRun: 6,
      modeBindings: [
        'com.example.Mail:builtin.email',
        'com.example.Chat:builtin.messages',
      ],
    });
    expect(loadDiscoveryState()?.completed).toEqual([
      'meeting', 'correction', 'voice_query', 'mode_binding',
    ]);
  });

  it('grandfathers only real usage and explicit configuration', () => {
    initializeGrandfatheredDiscovery({
      ...emptyEvidence(),
      meetings: 1,
      correctionsTaught: 1,
      voiceQueryConfigured: true,
      modeBindings: ['com.example.Editor:builtin.technical'],
    });
    expect(loadDiscoveryState()?.completed).toEqual([
      'meeting', 'correction', 'voice_query', 'mode_binding',
    ]);
    resetDiscovery();
    initializeGrandfatheredDiscovery(emptyEvidence());
    expect(loadDiscoveryState()?.completed).toEqual([]);
  });

  it('lowers a baseline after statistics reset so the next action completes', () => {
    initializeDiscoveryAfterSetup({ ...emptyEvidence(), meetings: 12 });
    syncDiscoveryEvidence(emptyEvidence());
    expect(loadDiscoveryState()?.completed).toEqual([]);
    syncDiscoveryEvidence({ ...emptyEvidence(), meetings: 1 });
    expect(loadDiscoveryState()?.completed).toContain('meeting');
  });

  it('persists checklist visibility and once-only hint delivery across reloads', () => {
    initializeDiscoveryAfterSetup(emptyEvidence());
    completeChecklistItem('command_palette');
    dismissDiscoveryChecklist();
    enqueueDiscoveryHint('browser_modes');
    enqueueDiscoveryHint('browser_modes');

    expect(loadDiscoveryState()).toMatchObject({
      checklistDismissed: true,
      completed: ['command_palette'],
      pendingHints: ['browser_modes'],
    });
    reopenDiscoveryChecklist();
    dismissDiscoveryHint('browser_modes');
    enqueueDiscoveryHint('browser_modes');
    expect(loadDiscoveryState()).toMatchObject({
      checklistDismissed: false,
      pendingHints: [],
      dismissedHints: ['browser_modes'],
    });

    resetDiscovery();
    expect(localStorage.getItem(DISCOVERY_STORAGE_KEY)).toBeNull();
  });

  it('clears discovery whenever either setup entry point calls the shared reset', () => {
    markOnboardingComplete();
    initializeDiscoveryAfterSetup(emptyEvidence());
    resetOnboarding();
    expect(isOnboardingComplete()).toBe(false);
    expect(loadDiscoveryState()).toBeNull();
  });
});
