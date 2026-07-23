import { describe, expect, it } from 'vitest';
import {
  HOLD_MIN_MS,
  INITIAL_TRANSFORM_FLOW_STATE,
  reduceTransformFlow,
  type TransformFlowState,
} from './transformFlow';

describe('reduceTransformFlow', () => {
  it('starts a capture on the first press', () => {
    const step = reduceTransformFlow(INITIAL_TRANSFORM_FLOW_STATE, { type: 'pressed', now: 1000, transformPassId: 7 });
    expect(step.command).toBe('start_transform_capture');
    expect(step.state).toEqual({ holding: true, pressedAt: 1000, transformPassId: 7 });
    expect(step.transformPassId).toBe(7);
    expect(step.ignored).toBeUndefined();
  });

  it('ignores a double-press while already holding', () => {
    const held: TransformFlowState = { holding: true, pressedAt: 1000, transformPassId: 7 };
    const step = reduceTransformFlow(held, { type: 'pressed', now: 1050, transformPassId: 8 });
    expect(step.command).toBeNull();
    expect(step.ignored).toBe('double_press');
    // State is unchanged — the original hold is preserved.
    expect(step.state).toEqual(held);
  });

  it('finishes the instruction on a normal-length release', () => {
    const held: TransformFlowState = { holding: true, pressedAt: 1000, transformPassId: 7 };
    const step = reduceTransformFlow(held, { type: 'released', now: 1000 + HOLD_MIN_MS + 1, transformPassId: 7 });
    expect(step.command).toBe('finish_transform_instruction');
    expect(step.state).toEqual({ holding: false, pressedAt: null, transformPassId: null });
    expect(step.transformPassId).toBe(7);
  });

  it('cancels silently on a too-short tap', () => {
    const held: TransformFlowState = { holding: true, pressedAt: 1000, transformPassId: 7 };
    const step = reduceTransformFlow(held, { type: 'released', now: 1000 + HOLD_MIN_MS - 1, transformPassId: 7 });
    expect(step.command).toBe('cancel_transform');
    expect(step.state).toEqual({ holding: false, pressedAt: null, transformPassId: null });
  });

  it('exactly HOLD_MIN_MS counts as a real instruction (not a tap)', () => {
    const held: TransformFlowState = { holding: true, pressedAt: 1000, transformPassId: 7 };
    const step = reduceTransformFlow(held, { type: 'released', now: 1000 + HOLD_MIN_MS, transformPassId: 7 });
    expect(step.command).toBe('finish_transform_instruction');
  });

  it('tolerates a stray release with no active hold', () => {
    const step = reduceTransformFlow(INITIAL_TRANSFORM_FLOW_STATE, { type: 'released', now: 5000, transformPassId: 7 });
    expect(step.command).toBeNull();
    expect(step.ignored).toBe('stray_release');
    expect(step.state).toEqual(INITIAL_TRANSFORM_FLOW_STATE);
  });

  it('reset recovers from a missing release, so the next press works again', () => {
    // A hold whose release event never arrived (listener stopped mid-hold).
    const stuck: TransformFlowState = { holding: true, pressedAt: 1000, transformPassId: 7 };
    // Without a reset, a new press would be ignored as a double-press:
    expect(reduceTransformFlow(stuck, { type: 'pressed', now: 9000, transformPassId: 8 }).ignored).toBe('double_press');
    // The hook dispatches `reset` when the listener (re)starts.
    const afterReset = reduceTransformFlow(stuck, { type: 'reset' });
    expect(afterReset.state).toEqual(INITIAL_TRANSFORM_FLOW_STATE);
    // Now the next press starts a fresh capture.
    const step = reduceTransformFlow(afterReset.state, { type: 'pressed', now: 9100, transformPassId: 9 });
    expect(step.command).toBe('start_transform_capture');
  });

  it('a full press/release/press cycle drives start -> finish -> start', () => {
    let state = INITIAL_TRANSFORM_FLOW_STATE;
    let step = reduceTransformFlow(state, { type: 'pressed', now: 0, transformPassId: 1 });
    expect(step.command).toBe('start_transform_capture');
    state = step.state;

    step = reduceTransformFlow(state, { type: 'released', now: 1000, transformPassId: 1 });
    expect(step.command).toBe('finish_transform_instruction');
    state = step.state;

    step = reduceTransformFlow(state, { type: 'pressed', now: 2000, transformPassId: 2 });
    expect(step.command).toBe('start_transform_capture');
  });

  it('rejects a stale release without cancelling the active hold', () => {
    const held: TransformFlowState = {
      holding: true,
      pressedAt: 1000,
      transformPassId: 11,
    };
    const step = reduceTransformFlow(held, {
      type: 'released',
      now: 1500,
      transformPassId: 10,
    });
    expect(step.command).toBeNull();
    expect(step.ignored).toBe('stale_release');
    expect(step.state).toEqual(held);
  });
});
