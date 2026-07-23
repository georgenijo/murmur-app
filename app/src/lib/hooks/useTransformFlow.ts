import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DEFAULT_SETTINGS } from '../settings';
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
  /** Selected microphone device id (same contract as start_native_recording). */
  microphone?: string;
}

interface TransformKeyPayload {
  transformPassId: number;
}

function isTransformKeyPayload(value: unknown): value is TransformKeyPayload {
  if (!value || typeof value !== 'object') return false;
  const passId = (value as Record<string, unknown>).transformPassId;
  return typeof passId === 'number' && Number.isSafeInteger(passId) && passId > 0;
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
  microphone,
}: UseTransformFlowProps) {
  const stateRef = useRef<TransformFlowState>(INITIAL_TRANSFORM_FLOW_STATE);
  const microphoneRef = useRef(microphone);
  useEffect(() => {
    microphoneRef.current = microphone;
  }, [microphone]);

  useEffect(() => {
    if (!enabled || !initialized || !accessibilityGranted || !transformHoldKey) return;

    let cancelled = false;
    let unlistenPressed: (() => void) | null = null;
    let unlistenReleased: (() => void) | null = null;

    const deviceNameArg = () => {
      const mic = microphoneRef.current;
      return mic && mic !== DEFAULT_SETTINGS.microphone ? mic : null;
    };

    const dispatch = (input: TransformFlowInput) => {
      const step = reduceTransformFlow(stateRef.current, input);
      stateRef.current = step.state;
      if (step.ignored) {
        flog.info('transform-flow', 'input ignored', { reason: step.ignored, input: input.type });
        return;
      }
      if (step.command) {
        if (step.transformPassId === null) {
          flog.warn('transform-flow', 'command missing pass id', { command: step.command });
          return;
        }
        const args = step.command === 'start_transform_capture'
          ? {
              deviceName: deviceNameArg(),
              transformPassId: step.transformPassId,
            }
          : { transformPassId: step.transformPassId };
        invoke(step.command, args).catch(() => {
          // Rust emits the correlated stable error code. Do not duplicate a
          // raw native error string into frontend logs.
          flog.warn('transform-flow', 'command failed', {
            command: step.command,
            transform_pass_id: step.transformPassId,
          });
        });
      }
    };

    const setup = async () => {
      // A (re)start of the listener resets the reducer — a hold that lost its
      // release event (listener torn down mid-hold) must not wedge the next press.
      // If we were mid-hold, cancel the backend so Listening + live mic is not
      // left running with no release coming (C2 finding 5).
      if (stateRef.current.holding) {
        invoke('cancel_transform', {
          transformPassId: stateRef.current.transformPassId,
        }).catch(() => {});
      }
      stateRef.current = INITIAL_TRANSFORM_FLOW_STATE;

      unlistenPressed = await listen<unknown>('transform-key-pressed', (event) => {
        if (!isTransformKeyPayload(event.payload)) {
          flog.warn('transform-flow', 'invalid transform-key-pressed payload');
          return;
        }
        dispatch({
          type: 'pressed',
          now: Date.now(),
          transformPassId: event.payload.transformPassId,
        });
      });
      if (cancelled) { unlistenPressed(); return; }

      unlistenReleased = await listen<unknown>('transform-key-released', (event) => {
        if (!isTransformKeyPayload(event.payload)) {
          flog.warn('transform-flow', 'invalid transform-key-released payload');
          return;
        }
        dispatch({
          type: 'released',
          now: Date.now(),
          transformPassId: event.payload.transformPassId,
        });
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
      // Mid-hold cleanup: backend is Listening with a live mic and no release
      // will arrive after the listener stops — cancel so we do not wedge.
      if (stateRef.current.holding) {
        invoke('cancel_transform', {
          transformPassId: stateRef.current.transformPassId,
        }).catch(() => {});
        stateRef.current = INITIAL_TRANSFORM_FLOW_STATE;
      }
    };
  }, [enabled, initialized, accessibilityGranted, transformHoldKey]);
}
