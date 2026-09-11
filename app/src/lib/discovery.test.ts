import { beforeEach, describe, expect, it } from 'vitest';
import {
  DISCOVERY_STORAGE_KEY,
  checklistDestination,
  completeChecklistItem,
  dismissDiscoveryChecklist,
  dismissDiscoveryHint,
  enqueueDiscoveryHint,
  hintDestination,
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
      'transform', 'meeting', 'correction', 'voice_query', 'mode_binding',
    ]);
  });

  it('grandfathers only real usage and explicit configuration', () => {
    initializeGrandfatheredDiscovery({
      ...emptyEvidence(),
      transforms: 1,
      correctionsTaught: 1,
      voiceQueryConfigured: true,
      modeBindings: ['com.example.Editor:builtin.technical'],
    });
    expect(loadDiscoveryState()?.completed).toEqual([
      'transform', 'correction', 'voice_query', 'mode_binding',
    ]);
    resetDiscovery();
    initializeGrandfatheredDiscovery(emptyEvidence());
    expect(loadDiscoveryState()?.completed).toEqual([]);
  });

  it('lowers a baseline after statistics reset so the next action completes', () => {
    initializeDiscoveryAfterSetup({ ...emptyEvidence(), transforms: 12 });
    syncDiscoveryEvidence(emptyEvidence());
    expect(loadDiscoveryState()?.completed).toEqual([]);
    syncDiscoveryEvidence({ ...emptyEvidence(), transforms: 1 });
    expect(loadDiscoveryState()?.completed).toContain('transform');
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

  it('wires every checklist and hint to its exact destination', () => {
    expect(checklistDestination('command_palette')).toEqual({ kind: 'palette' });
    expect(checklistDestination('mode_binding')).toEqual({ kind: 'settings', page: 'modes', target: 'modes' });
    expect(checklistDestination('transform')).toEqual({ kind: 'settings', page: 'ai-transform', target: 'transform-practice' });
    expect(checklistDestination('voice_query')).toEqual({ kind: 'settings', page: 'ai-query', target: 'voice-query-provider' });
    expect(checklistDestination('meeting')).toEqual({ kind: 'main', page: 'meetings' });
    expect(checklistDestination('correction')).toEqual({ kind: 'teach' });
    expect(checklistDestination('shortcuts')).toEqual({ kind: 'settings', page: 'shortcuts', target: 'shortcuts' });
    expect(hintDestination('double_tap_timing')).toEqual({ kind: 'settings', page: 'recording', target: 'hotkey-feedback' });
    expect(hintDestination('correct_and_teach')).toEqual({ kind: 'teach' });
    expect(hintDestination('browser_modes')).toEqual({ kind: 'settings', page: 'modes', target: 'modes' });
    expect(hintDestination('meeting_summaries')).toEqual({ kind: 'main', page: 'meetings' });
  });

  it('clears discovery whenever either setup entry point calls the shared reset', () => {
    markOnboardingComplete();
    initializeDiscoveryAfterSetup(emptyEvidence());
    resetOnboarding();
    expect(isOnboardingComplete()).toBe(false);
    expect(loadDiscoveryState()).toBeNull();
  });
});
