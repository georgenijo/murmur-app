import { describe, expect, it, vi } from 'vitest';
import { CUE_PLAYOUT_MARGIN_MS, clampCueVolume, cueDurationMs, playSoundCue } from './soundCues';

describe('sound cues', () => {
  it('clamps persisted and preview volume to a bounded percentage', () => {
    expect(clampCueVolume(-2)).toBe(0);
    expect(clampCueVolume(42.6)).toBe(43);
    expect(clampCueVolume(500)).toBe(100);
    expect(clampCueVolume(Number.NaN)).toBe(45);
  });

  it('schedules output nodes without accepting or returning microphone samples', () => {
    const oscillator = {
      type: 'sine',
      frequency: { setValueAtTime: vi.fn() },
      connect: vi.fn(),
      start: vi.fn(),
      stop: vi.fn(),
    };
    const gain = {
      gain: {
        setValueAtTime: vi.fn(),
        exponentialRampToValueAtTime: vi.fn(),
      },
      connect: vi.fn(),
    };
    class FakeAudioContext {
      currentTime = 1;
      destination = {};
      createOscillator = () => oscillator;
      createGain = () => gain;
      close = vi.fn();
    }
    vi.stubGlobal('AudioContext', FakeAudioContext);
    expect(playSoundCue.length).toBe(2);
    expect(playSoundCue('start', 45)).toBeInstanceOf(Promise);
    expect(oscillator.start).toHaveBeenCalledOnce();
    expect(gain.connect).toHaveBeenCalledWith(expect.anything());
    vi.unstubAllGlobals();
  });

  it('creates no audio context or output nodes at zero volume, and settles at once', async () => {
    const AudioContext = vi.fn();
    vi.stubGlobal('AudioContext', AudioContext);
    await expect(playSoundCue('failure', 0)).resolves.toBeUndefined();
    expect(AudioContext).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });

  function fakeContext(outputLatency: number) {
    const oscillators: Array<{ onended: (() => void) | null }> = [];
    const close = vi.fn(async () => {});
    class FakeAudioContext {
      currentTime = 0;
      outputLatency = outputLatency;
      baseLatency = 0;
      destination = {};
      createOscillator = () => {
        const oscillator = {
          type: 'sine', onended: null, frequency: { setValueAtTime: vi.fn() },
          connect: vi.fn(), start: vi.fn(), stop: vi.fn(),
        };
        oscillators.push(oscillator);
        return oscillator;
      };
      createGain = () => ({ gain: { setValueAtTime: vi.fn(), exponentialRampToValueAtTime: vi.fn() }, connect: vi.fn() });
      close = close;
    }
    vi.stubGlobal('AudioContext', FakeAudioContext);
    return { oscillators, close };
  }

  it('settles only after the last tone ends on the audio clock plus output latency and margin', async () => {
    vi.useFakeTimers();
    try {
      const { oscillators, close } = fakeContext(0.012);
      let settled = false;
      void playSoundCue('success', 45).then(() => { settled = true; });
      // Two tones; the later-ending one owns the play-out signal.
      expect(oscillators).toHaveLength(2);
      expect(oscillators[0].onended).toBeNull();
      await vi.advanceTimersByTimeAsync(500);
      expect(settled).toBe(false);
      oscillators[1].onended?.();
      await vi.advanceTimersByTimeAsync(12 + CUE_PLAYOUT_MARGIN_MS - 1);
      expect(settled).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(settled).toBe(true);
      expect(close).toHaveBeenCalledOnce();
    } finally {
      vi.useRealTimers();
      vi.unstubAllGlobals();
    }
  });

  it('settles on a fallback when the audio clock never reports the end', async () => {
    vi.useFakeTimers();
    try {
      fakeContext(0);
      let settled = false;
      void playSoundCue('start', 45).then(() => { settled = true; });
      await vi.advanceTimersByTimeAsync(cueDurationMs('start') + 999);
      expect(settled).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(settled).toBe(true);
    } finally {
      vi.useRealTimers();
      vi.unstubAllGlobals();
    }
  });

  it('keeps the start cue well inside the backend mute cap', () => {
    // output_quieting.rs START_CUE_CAP (500 ms) mutes anyway if the play-out
    // report never arrives. The cue, the 200 ms latency bound, and the margin
    // must fit with room for a slow webview; a longer cue fails here.
    const START_CUE_CAP_MS = 500;
    expect(cueDurationMs('start')).toBe(70);
    expect(cueDurationMs('start') + 200 + CUE_PLAYOUT_MARGIN_MS).toBeLessThan(START_CUE_CAP_MS);
  });
});
