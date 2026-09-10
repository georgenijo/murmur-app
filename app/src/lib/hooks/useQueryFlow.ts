import { useEffect, useLayoutEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DEFAULT_SETTINGS, type QueryKey, type SmartAutoMicrophoneRequest } from '../settings';
import { validateQueryCommand, type QueryCommandConfig } from '../queryProviders';
import { isQueryUsage } from '../queryUsage';
import { isHiddenPayload, isQueryStatePayload, isValidPassId } from '../queryReview';
import type { QueryCompletion } from '../stats';
import { flog } from '../log';
import { queryConfigurationMessage } from '../voiceQuerySettings';
interface QueryTogglePayload {
  queryPassId: number;
  action: 'start' | 'stop';
}

interface TrackedQueryPass {
  provider: QueryCommandConfig['provider'];
  completed: boolean;
}

export type QuerySetupStatus =
  | { state: 'ready' }
  | {
    state: 'failed';
    phase: 'command_validation' | 'listener_start';
    message: string;
  };

interface UseQueryFlowProps {
  enabled: boolean;
  initialized: boolean;
  accessibilityGranted: boolean | null;
  queryHotkey: QueryKey | null;
  microphone?: string;
  smartAuto?: SmartAutoMicrophoneRequest | null;
  automaticallyCopyAnswers: boolean;
  command: QueryCommandConfig;
  onQueryCompleted?: (completion: QueryCompletion) => void;
  onSetupStatusChange?: (status: QuerySetupStatus) => void;
}

function isTogglePayload(value: unknown): value is QueryTogglePayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return isValidPassId(payload.queryPassId)
    && (payload.action === 'start' || payload.action === 'stop');
}

export function useQueryFlow({
  enabled,
  initialized,
  accessibilityGranted,
  queryHotkey,
  microphone,
  smartAuto = null,
  automaticallyCopyAnswers,
  command,
  onQueryCompleted,
  onSetupStatusChange,
}: UseQueryFlowProps) {
  const activePassRef = useRef<number | null>(null);
  const trackedPassesRef = useRef(new Map<number, TrackedQueryPass>());
  const commandRef = useRef(command);
  const microphoneRef = useRef(microphone);
  const smartAutoRef = useRef(smartAuto);
  const automaticallyCopyAnswersRef = useRef(automaticallyCopyAnswers);
  const onQueryCompletedRef = useRef(onQueryCompleted);
  const onSetupStatusChangeRef = useRef(onSetupStatusChange);
  const setupGenerationRef = useRef(0);
  const terminalListenersReadyRef = useRef<Promise<void> | null>(null);
  const terminalStateUnlistenRef = useRef<(() => void) | null>(null);
  const terminalHiddenUnlistenRef = useRef<(() => void) | null>(null);
  const terminalListenerGenerationRef = useRef(0);
  // Native events can arrive after React commits new settings but before
  // passive effects run. Refresh every value read by those callbacks during
  // the synchronous layout phase so a newly selected provider cannot start a
  // pass with the previous provider's command.
  useLayoutEffect(() => { commandRef.current = command; }, [command]);
  useLayoutEffect(() => { microphoneRef.current = microphone; }, [microphone]);
  useLayoutEffect(() => { smartAutoRef.current = smartAuto; }, [smartAuto]);
  useLayoutEffect(() => { automaticallyCopyAnswersRef.current = automaticallyCopyAnswers; }, [automaticallyCopyAnswers]);
  useLayoutEffect(() => { onQueryCompletedRef.current = onQueryCompleted; }, [onQueryCompleted]);
  useLayoutEffect(() => { onSetupStatusChangeRef.current = onSetupStatusChange; }, [onSetupStatusChange]);
  useLayoutEffect(() => {
    setupGenerationRef.current += 1;
  }, [enabled, initialized, accessibilityGranted, queryHotkey]);

  const completeTrackedPass = (
    queryPassId: number,
    completion: Omit<QueryCompletion, 'provider'>,
  ) => {
    const tracked = trackedPassesRef.current.get(queryPassId);
    if (!tracked || tracked.completed) return false;
    tracked.completed = true;
    onQueryCompletedRef.current?.({ provider: tracked.provider, ...completion });
    return true;
  };

  const releaseTrackedPass = (queryPassId: number) => {
    if (activePassRef.current === queryPassId) activePassRef.current = null;
    trackedPassesRef.current.delete(queryPassId);
  };

  const ensureTerminalListeners = (): Promise<void> => {
    if (terminalStateUnlistenRef.current && terminalHiddenUnlistenRef.current) {
      return Promise.resolve();
    }
    if (terminalListenersReadyRef.current) return terminalListenersReadyRef.current;

    const listenerGeneration = terminalListenerGenerationRef.current;
    const ownsListeners = () => (
      terminalListenerGenerationRef.current === listenerGeneration
    );
    const attempt = (async () => {
      let unlistenState: (() => void) | null = null;
      let unlistenHidden: (() => void) | null = null;
      try {
        unlistenState = await listen<unknown>('query-state-changed', (event) => {
          if (!ownsListeners() || !isQueryStatePayload(event.payload)) return;
          const payload = event.payload;
          if (payload.state !== 'ready' && payload.state !== 'failed') return;
          const completed = completeTrackedPass(payload.queryPassId, {
            succeeded: payload.state === 'ready',
            errorCode: payload.errorCode,
            usage: isQueryUsage(payload.usage) ? payload.usage : null,
          });
          if (completed && activePassRef.current !== payload.queryPassId) {
            trackedPassesRef.current.delete(payload.queryPassId);
          }
        });
        if (!ownsListeners()) {
          unlistenState();
          return;
        }

        unlistenHidden = await listen<unknown>('query-review-hidden', (event) => {
          if (!ownsListeners() || !isHiddenPayload(event.payload)) return;
          const { queryPassId } = event.payload;
          if (!trackedPassesRef.current.has(queryPassId)) return;
          completeTrackedPass(queryPassId, {
            succeeded: false,
            errorCode: 'cancelled',
            usage: null,
          });
          releaseTrackedPass(queryPassId);
        });
        if (!ownsListeners()) {
          unlistenState();
          unlistenHidden();
          return;
        }
        terminalStateUnlistenRef.current = unlistenState;
        terminalHiddenUnlistenRef.current = unlistenHidden;
      } catch (error) {
        unlistenState?.();
        unlistenHidden?.();
        throw error;
      }
    })();
    terminalListenersReadyRef.current = attempt;
    void attempt.then(
      () => {
        if (terminalListenersReadyRef.current === attempt) terminalListenersReadyRef.current = null;
      },
      () => {
        if (terminalListenersReadyRef.current === attempt) terminalListenersReadyRef.current = null;
      },
    );
    return attempt;
  };

  // Terminal accounting outlives the native-shortcut lifecycle. Disabling or
  // reconfiguring Voice Query cancels the current Rust pass, whose canonical
  // Ready/Failed/hidden event may arrive after that lifecycle effect cleans
  // up. Keeping these listeners mounted prevents command-response ordering
  // from turning an already-terminal pass into a synthetic cancellation.
  useEffect(() => {
    terminalListenerGenerationRef.current += 1;
    void ensureTerminalListeners().catch(() => {});

    return () => {
      terminalListenerGenerationRef.current += 1;
      terminalStateUnlistenRef.current?.();
      terminalHiddenUnlistenRef.current?.();
      terminalStateUnlistenRef.current = null;
      terminalHiddenUnlistenRef.current = null;
      terminalListenersReadyRef.current = null;
      activePassRef.current = null;
      trackedPassesRef.current.clear();
    };
  }, []);

  useEffect(() => {
    if (!enabled || !initialized || !accessibilityGranted || !queryHotkey) return;
    let disposed = false;
    let unlistenToggle: (() => void) | null = null;
    const setupGeneration = setupGenerationRef.current;
    const ownsSetup = () => (
      !disposed && setupGenerationRef.current === setupGeneration
    );
    const failSetup = (
      phase: Extract<QuerySetupStatus, { state: 'failed' }>['phase'],
      message: string,
    ) => {
      if (!ownsSetup()) return;
      unlistenToggle?.();
      unlistenToggle = null;
      flog.warn('query', 'voice-query preflight or listener setup failed');
      onSetupStatusChangeRef.current?.({ state: 'failed', phase, message });
    };

    const setup = async () => {
      // Install the completion observer before a toggle can start a pass. A
      // synchronous start failure must still be folded into usage exactly
      // once rather than landing in the listener-registration gap.
      try {
        await ensureTerminalListeners();
      } catch {
        failSetup(
          'listener_start',
          'Voice Query was turned off because its event listeners could not start. Try enabling it again. If it still fails, quit and reopen Murmur.',
        );
        return;
      }
      if (!ownsSetup()) return;

      try {
        unlistenToggle = await listen<unknown>('query-toggle', (event) => {
          if (!ownsSetup() || !isTogglePayload(event.payload)) return;
          const { queryPassId, action } = event.payload;
          if (action === 'start') {
            const immutableCommand = commandRef.current;
            for (const [trackedPassId, tracked] of trackedPassesRef.current) {
              if (tracked.completed && trackedPassId !== queryPassId) {
                trackedPassesRef.current.delete(trackedPassId);
              }
            }
            if (trackedPassesRef.current.has(queryPassId)) return;
            activePassRef.current = queryPassId;
            trackedPassesRef.current.set(queryPassId, {
              provider: immutableCommand.provider,
              completed: false,
            });
            const selectedMicrophone = microphoneRef.current;
            void invoke('start_query_capture', {
              queryPassId,
              deviceName: smartAutoRef.current ? null : selectedMicrophone && selectedMicrophone !== DEFAULT_SETTINGS.microphone
                ? selectedMicrophone
                : null,
              ...(smartAutoRef.current ? { smartAuto: smartAutoRef.current } : {}),
              automaticallyCopyAnswer: automaticallyCopyAnswersRef.current,
              command: immutableCommand,
            }).catch(() => {
              flog.warn('query', 'start command failed', { query_pass_id: queryPassId });
              void invoke('cancel_query', { queryPassId }).catch(() => {});
            });
            return;
          }
          if (activePassRef.current !== queryPassId) return;
          void invoke('finish_query_capture', { queryPassId }).catch(() => {
            flog.warn('query', 'finish command failed', { query_pass_id: queryPassId });
            void invoke('cancel_query', { queryPassId }).catch(() => {});
          });
        });
      } catch {
        failSetup(
          'listener_start',
          'Voice Query was turned off because its event listener could not start. Try enabling it again. If it still fails, quit and reopen Murmur.',
        );
        return;
      }
      if (!ownsSetup()) { unlistenToggle(); return; }

      try {
        // Preflight the exact provider, executable, argv, timeout, and
        // Rust-owned declared environment before arming the global shortcut.
        // A bad configuration therefore cannot wait until the first keypress
        // to fail.
        await validateQueryCommand(commandRef.current);
      } catch (error) {
        failSetup(
          'command_validation',
          `${queryConfigurationMessage(error)} Voice Query was turned off. Review the provider, choose Test, then enable it again.`,
        );
        return;
      }
      if (!ownsSetup()) return;
      try {
        await invoke('start_query_listener', { hotkey: queryHotkey });
      } catch {
        failSetup(
          'listener_start',
          'Voice Query was turned off because its shortcut could not start. Check Accessibility permission and shortcut conflicts, then enable it again.',
        );
        return;
      }
      if (ownsSetup()) onSetupStatusChangeRef.current?.({ state: 'ready' });
    };
    void setup();

    return () => {
      disposed = true;
      unlistenToggle?.();
      void invoke('stop_query_listener').catch(() => {});
      const passId = activePassRef.current;
      if (passId !== null) {
        // Rust owns the terminal outcome. The stable listeners above wait for
        // its pass-correlated Ready/Failed/hidden event even if this command
        // response settles first or rejects.
        void invoke('cancel_query', { queryPassId: passId }).catch(() => {});
      }
    };
  // Command changes are intentionally excluded: the start event snapshots
  // commandRef for the next pass, while an active pass keeps its Rust-owned
  // immutable command and must not be cancelled by a Settings rerender.
  }, [
    enabled,
    initialized,
    accessibilityGranted,
    queryHotkey,
  ]);
}
