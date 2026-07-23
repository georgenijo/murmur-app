// Pure reducer for the main-window transform hotkey driver (issue #312 PR-C2).
//
// The transform hotkey emits `transform-key-pressed` / `transform-key-released`
// exactly like the dictation hold-down hotkey. This reducer turns that
// press/release stream into the flow commands, with two edge rules the review
// flagged:
//
//  - **double-press ignore**: a second `pressed` while already holding (no
//    intervening `released`) is ignored — a key-repeat / bounce must not start
//    a second capture. (The backend independently ignores a start while a
//    transform is mid-flight, so a press during thinking/review is also safe.)
//  - **missing-release tolerance**: if a `released` never arrives (e.g. the
//    rdev listener was stopped mid-hold, per the B1 review note), the hook
//    dispatches `reset` when the listener (re)starts, clearing the stuck
//    `holding` so the next press works again.
//
// A release shorter than HOLD_MIN_MS is treated as an accidental tap: the flow
// is cancelled silently rather than transcribing empty audio.

export const HOLD_MIN_MS = 300;

export interface TransformFlowState {
  /** True between a `pressed` and its matching `released`. */
  holding: boolean;
  /** Timestamp (ms) of the current hold's press, or null when not holding. */
  pressedAt: number | null;
  /** Rust-allocated correlation ID for the physical hold. */
  transformPassId: number | null;
}

export const INITIAL_TRANSFORM_FLOW_STATE: TransformFlowState = {
  holding: false,
  pressedAt: null,
  transformPassId: null,
};

export type TransformFlowInput =
  | { type: 'pressed'; now: number; transformPassId: number }
  | { type: 'released'; now: number; transformPassId: number }
  | { type: 'reset' };

/** The Tauri command a step wants invoked, or null for a logged no-op. */
export type TransformFlowCommand =
  | 'start_transform_capture'
  | 'finish_transform_instruction'
  | 'cancel_transform';

export interface TransformFlowStep {
  state: TransformFlowState;
  command: TransformFlowCommand | null;
  /** Pass ID to send with the command, retained across release-state reset. */
  transformPassId: number | null;
  /** Human-readable reason a step was a no-op (for logging/tests). */
  ignored?: 'double_press' | 'stray_release' | 'stale_release';
}

export function reduceTransformFlow(
  state: TransformFlowState,
  input: TransformFlowInput,
): TransformFlowStep {
  switch (input.type) {
    case 'reset':
      return {
        state: INITIAL_TRANSFORM_FLOW_STATE,
        command: null,
        transformPassId: state.transformPassId,
      };

    case 'pressed': {
      // Double-press ignore: already holding from a prior, unreleased press.
      if (state.holding) {
        return {
          state,
          command: null,
          transformPassId: state.transformPassId,
          ignored: 'double_press',
        };
      }
      return {
        state: {
          holding: true,
          pressedAt: input.now,
          transformPassId: input.transformPassId,
        },
        command: 'start_transform_capture',
        transformPassId: input.transformPassId,
      };
    }

    case 'released': {
      // Stray release (no active hold) — tolerated, never wedges the flow.
      if (!state.holding) {
        return {
          state,
          command: null,
          transformPassId: null,
          ignored: 'stray_release',
        };
      }
      if (state.transformPassId !== input.transformPassId) {
        return {
          state,
          command: null,
          transformPassId: input.transformPassId,
          ignored: 'stale_release',
        };
      }
      const heldMs = state.pressedAt === null ? Infinity : input.now - state.pressedAt;
      const command: TransformFlowCommand =
        heldMs < HOLD_MIN_MS ? 'cancel_transform' : 'finish_transform_instruction';
      return {
        state: {
          holding: false,
          pressedAt: null,
          transformPassId: null,
        },
        command,
        transformPassId: input.transformPassId,
      };
    }

    default: {
      const _exhaustive: never = input;
      void _exhaustive;
      return { state, command: null, transformPassId: state.transformPassId };
    }
  }
}
