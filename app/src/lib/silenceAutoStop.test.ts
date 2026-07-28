import { describe, it, expect } from 'vitest';
import {
  initialSilenceState,
  MAX_SAMPLE_GAP_MS,
  MIN_SPEECH_LEVEL,
  MIN_SPEECH_MS,
  reduceSilenceSample,
  speechThreshold,
  type SilenceAutoStopState,
} from './silenceAutoStop';

const SILENCE_MS = 2000;
const LOUD = 0.2;
const QUIET = 0.001;

/** Feed a run of samples at 16ms intervals, returning the state and whether a
 *  stop was reported at any point. */
function feed(
  state: SilenceAutoStopState,
  levels: number[],
  startMs: number,
  stepMs = 16,
  silenceMs = SILENCE_MS,
): { state: SilenceAutoStopState; stops: number; stopAtMs: number | null } {
  let current = state;
  let stops = 0;
  let stopAtMs: number | null = null;
  levels.forEach((level, index) => {
    const atMs = startMs + index * stepMs;
    const result = reduceSilenceSample(current, { level, atMs }, silenceMs);
    current = result.state;
    if (result.stop) {
      stops += 1;
      if (stopAtMs === null) stopAtMs = atMs;
    }
  });
  return { state: current, stops, stopAtMs };
}

function repeat(level: number, ms: number, stepMs = 16): number[] {
  return new Array(Math.ceil(ms / stepMs)).fill(level);
}

describe('speechThreshold', () => {
  it('never drops below the absolute floor', () => {
    expect(speechThreshold(0)).toBe(MIN_SPEECH_LEVEL);
    expect(speechThreshold(0.01)).toBe(MIN_SPEECH_LEVEL);
  });

  it('rises with a loud recording so room tone cannot hold it open', () => {
    expect(speechThreshold(1)).toBeCloseTo(0.08);
    expect(speechThreshold(0.5)).toBeCloseTo(0.04);
  });
});

describe('reduceSilenceSample', () => {
  it('does nothing until speech has been heard', () => {
    const { state, stops } = feed(initialSilenceState(), repeat(QUIET, 10_000), 0);
    expect(stops).toBe(0);
    expect(state.armed).toBe(false);
    expect(state.silenceMs).toBe(0);
  });

  it('arms only after the minimum cumulative speech', () => {
    const short = feed(initialSilenceState(), repeat(LOUD, MIN_SPEECH_MS / 2), 0);
    expect(short.state.armed).toBe(false);
    const enough = feed(short.state, repeat(LOUD, MIN_SPEECH_MS), 1000);
    expect(enough.state.armed).toBe(true);
  });

  it('accumulates speech across pauses rather than requiring it to be continuous', () => {
    let state = initialSilenceState();
    for (let i = 0; i < 4; i++) {
      state = feed(state, repeat(LOUD, 150), i * 1000).state;
      state = feed(state, repeat(QUIET, 300), i * 1000 + 200).state;
    }
    expect(state.armed).toBe(true);
  });

  it('stops after the configured trailing silence', () => {
    const spoken = feed(initialSilenceState(), repeat(LOUD, 1000), 0);
    const { stops, stopAtMs } = feed(spoken.state, repeat(QUIET, 4000), 1000);
    expect(stops).toBe(1);
    expect(stopAtMs).toBeGreaterThanOrEqual(1000 + SILENCE_MS);
    expect(stopAtMs).toBeLessThan(1000 + SILENCE_MS + 100);
  });

  it('reports the stop exactly once and latches afterwards', () => {
    const spoken = feed(initialSilenceState(), repeat(LOUD, 1000), 0);
    const silent = feed(spoken.state, repeat(QUIET, 6000), 1000);
    expect(silent.stops).toBe(1);
    expect(silent.state.stopped).toBe(true);
    // Further samples — including loud ones — change nothing.
    const after = feed(silent.state, [...repeat(LOUD, 500), ...repeat(QUIET, 5000)], 20_000);
    expect(after.stops).toBe(0);
    expect(after.state).toBe(silent.state);
  });

  it('resets the silence run whenever speech resumes', () => {
    let state = feed(initialSilenceState(), repeat(LOUD, 1000), 0).state;
    state = feed(state, repeat(QUIET, 1900), 1000).state;
    expect(state.silenceMs).toBeGreaterThan(1800);
    state = feed(state, repeat(LOUD, 100), 3000).state;
    expect(state.silenceMs).toBe(0);
    const { stops } = feed(state, repeat(QUIET, 1900), 3200);
    expect(stops).toBe(0);
  });

  it('is disabled when the configured silence is zero or negative', () => {
    for (const disabled of [0, -1]) {
      const spoken = feed(initialSilenceState(), repeat(LOUD, 1000), 0, 16, disabled);
      const { stops, state } = feed(spoken.state, repeat(QUIET, 10_000), 1000, 16, disabled);
      expect(stops).toBe(0);
      expect(state).toBe(spoken.state);
    }
  });

  it('never charges more than the gap cap to a single sample', () => {
    const spoken = feed(initialSilenceState(), repeat(LOUD, 1000), 0);
    // One sample after a two-minute stall (suspended window) must not by itself
    // satisfy a two-second silence budget.
    const stalled = reduceSilenceSample(spoken.state, { level: QUIET, atMs: 121_000 }, SILENCE_MS);
    expect(stalled.stop).toBe(false);
    expect(stalled.state.silenceMs).toBe(MAX_SAMPLE_GAP_MS);
  });

  it('ignores backwards timestamps instead of accumulating negative time', () => {
    const spoken = feed(initialSilenceState(), repeat(LOUD, 1000), 0);
    const backwards = reduceSilenceSample(spoken.state, { level: QUIET, atMs: 0 }, SILENCE_MS);
    expect(backwards.state.silenceMs).toBe(0);
    expect(backwards.stop).toBe(false);
  });

  it('treats non-finite levels as silence', () => {
    const spoken = feed(initialSilenceState(), repeat(LOUD, 1000), 0);
    const result = reduceSilenceSample(spoken.state, { level: Number.NaN, atMs: 1016 }, SILENCE_MS);
    expect(result.state.silenceMs).toBeGreaterThan(0);
    expect(result.state.peak).toBe(spoken.state.peak);
  });

  it('does not arm on a microphone that never clears the absolute floor', () => {
    const whisperQuiet = MIN_SPEECH_LEVEL - 0.001;
    const { state, stops } = feed(initialSilenceState(), repeat(whisperQuiet, 30_000), 0);
    expect(state.armed).toBe(false);
    expect(stops).toBe(0);
  });

  it('treats room tone as silence once the speaker has been loud', () => {
    const roomTone = 0.02; // above the absolute floor, below 8% of a 0.5 peak
    let state = feed(initialSilenceState(), repeat(0.5, 1000), 0).state;
    expect(state.armed).toBe(true);
    const { stops } = feed(state, repeat(roomTone, 3000), 1000);
    expect(stops).toBe(1);
  });
});
