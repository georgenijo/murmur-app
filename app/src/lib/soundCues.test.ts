import { describe, expect, it, vi } from 'vitest';
import { clampCueVolume, playSoundCue } from './soundCues';

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
    expect(playSoundCue('start', 45)).toBeUndefined();
    expect(oscillator.start).toHaveBeenCalledOnce();
    expect(gain.connect).toHaveBeenCalledWith(expect.anything());
    vi.unstubAllGlobals();
  });
});
