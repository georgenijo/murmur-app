import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import {
  INITIAL_TRANSFORM_FLOW_STATE,
  reduceTransformFlow,
  type TransformFlowInput,
  type TransformFlowState,
} from '../transformFlow';

interface UseTransformFlowProps {
  /** transformHoldKey configured AND hotkeys armed. */
  enabled: boolean;
  initialized: boolean;
  accessibilityGranted: boolean | null;
  /** The configured transform hotkey (one of TransformKey), or null=disabled. */
  transformHoldKey: string | null;
}

/**
 * Main-window driver for the AX-selection transform flow (issue #312 PR-C2).
 *
 * Mirrors `useHoldDownToggle`'s listen-and-call-command pattern: it starts the
 * independent transform rdev listener, subscribes to `transform-key-pressed` /
 * `transform-key-released`, and turns that press/release stream into the flow
 * commands via the pure `reduceTransformFlow` reducer (double-press ignore,
 * short-tap cancel, and missing-release tolerance — see that module).
 *
 * Always call this hook (gate its *effects* on `enabled`, per the Rules of
 * Hooks) — same discipline as the dictation hotkey hooks. The popover window
 * drives Approve/Retry/Cancel/Undo separately (`useTransformReviewDriver`).
 */
export function useTransformFlow({
  enabled,
  initialized,
  accessibilityGranted,
  transformHoldKey,
}: UseTransformFlowProps) {
  const stateRef = useRef<TransformFlowState>(INITIAL_TRANSFORM_FLOW_STATE);

  useEffect(() => {
    if (!enabled || !initialized || !accessibilityGranted || !transformHoldKey) return;

    let cancelled = false;
    let unlistenPressed: (() => void) | null = null;
    let unlistenReleased: (() => void) | null = null;

    const dispatch = (input: TransformFlowInput) => {
      const step = reduceTransformFlow(stateRef.current, input);
      stateRef.current = step.state;
      if (step.ignored) {
        flog.info('transform-flow', 'input ignored', { reason: step.ignored, input: input.type });
        return;
      }
      if (step.command) {
        invoke(step.command).catch((e) => {
          flog.warn('transform-flow', 'command failed', { command: step.command, error: String(e) });
        });
      }
    };

    const setup = async () => {
      // A (re)start of the listener resets the reducer — a hold that lost its
      // release event (listener torn down mid-hold) must not wedge the next press.
      stateRef.current = INITIAL_TRANSFORM_FLOW_STATE;

      unlistenPressed = await listen('transform-key-pressed', () => {
        dispatch({ type: 'pressed', now: Date.now() });
      });
      if (cancelled) { unlistenPressed(); return; }

      unlistenReleased = await listen('transform-key-released', () => {
        dispatch({ type: 'released', now: Date.now() });
      });
      if (cancelled) { unlistenPressed(); unlistenReleased(); return; }

      try {
        await invoke('start_transform_listener', { hotkey: transformHoldKey });
        if (cancelled) {
          invoke('stop_transform_listener').catch(() => {});
        }
      } catch (err) {
        flog.warn('transform-flow', 'failed to start transform listener', { error: String(err) });
      }
    };

    setup();

    return () => {
      cancelled = true;
      unlistenPressed?.();
      unlistenReleased?.();
      invoke('stop_transform_listener').catch(() => {});
    };
  }, [enabled, initialized, accessibilityGranted, transformHoldKey]);
}
