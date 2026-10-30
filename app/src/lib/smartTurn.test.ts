import { describe, expect, it } from 'vitest';
import { initialSilenceState, reduceSilenceSample, type SilenceAutoStopState } from './silenceAutoStop';
import { SMART_TURN_CHECK_MS, SMART_TURN_THRESHOLD, smartTurnCheckDue, smartTurnVerdict } from './smartTurn';

/** Fold `ms` of samples at `level`, 16 ms apart. */
function feed(state: SilenceAutoStopState, level: number, ms: number, at: { now: number }) {
  let next = state;
  for (let elapsed = 0; elapsed < ms; elapsed += 16) {
    at.now += 16;
    next = reduceSilenceSample(next, { level, atMs: at.now }, 4000).state;
  }
  return next;
}

const scored = (probability: number) => ({ outcome: 'scored' as const, recordingId: 1, probability, inferenceMs: 5 });

describe('Smart Turn decisions', () => {
  it('counts one pause per silence run after speech', () => {
    const at = { now: 0 };
    let state = feed(initialSilenceState(), 0.0, 500, at);
    expect(state.pause).toBe(0);
    state = feed(state, 0.3, 600, at);
    state = feed(state, 0.0, 200, at);
    expect(state.pause).toBe(1);
    state = feed(state, 0.0, 600, at);
    expect(state.pause).toBe(1);
    state = feed(state, 0.3, 200, at);
    state = feed(state, 0.0, 200, at);
    expect(state.pause).toBe(2);
  });

  it('is due once the pause reaches the check point, once per pause', () => {
    const at = { now: 0 };
    let state = feed(feed(initialSilenceState(), 0.3, 600, at), 0.0, SMART_TURN_CHECK_MS - 40, at);
    expect(smartTurnCheckDue(state, 0)).toBe(false);
    state = feed(state, 0.0, 60, at);
    expect(smartTurnCheckDue(state, 0)).toBe(true);
    expect(smartTurnCheckDue(state, state.pause)).toBe(false);
  });

  it('is never due before speech or after a stop', () => {
    const at = { now: 0 };
    expect(smartTurnCheckDue(feed(initialSilenceState(), 0.0, 2000, at), 0)).toBe(false);
    const state = feed(feed(initialSilenceState(), 0.3, 600, at), 0.0, 400, at);
    expect(smartTurnCheckDue({ ...state, stopped: true }, 0)).toBe(false);
  });

  it('stops only at or over the threshold, and only for the pause still running', () => {
    const at = { now: 0 };
    const state = feed(feed(initialSilenceState(), 0.3, 600, at), 0.0, 400, at);
    expect(smartTurnVerdict(state, state.pause, scored(SMART_TURN_THRESHOLD))).toBe('stop');
    expect(smartTurnVerdict(state, state.pause, scored(SMART_TURN_THRESHOLD - 0.01))).toBe('continue');
    expect(smartTurnVerdict(state, state.pause, scored(Number.NaN))).toBe('continue');
    expect(smartTurnVerdict(state, state.pause, { outcome: 'unavailable', reason: 'busy' })).toBe('continue');
    expect(smartTurnVerdict(state, state.pause - 1, scored(0.99))).toBe('stale');
    expect(smartTurnVerdict({ ...state, silenceMs: 0 }, state.pause, scored(0.99))).toBe('stale');
    expect(smartTurnVerdict({ ...state, stopped: true }, state.pause, scored(0.99))).toBe('stale');
  });
});
