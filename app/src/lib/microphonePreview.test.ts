import { describe, expect, it } from 'vitest';
import {
  microphoneClassificationLabel,
  microphoneLevelPercent,
  microphonePeakPercent,
} from './microphonePreview';

describe('microphone preview presentation', () => {
  it('maps low RMS into visible meter space while bounding malformed values', () => {
    expect(microphoneLevelPercent(0)).toBe(0);
    expect(microphoneLevelPercent(0.01)).toBe(10);
    expect(microphoneLevelPercent(1)).toBe(100);
    expect(microphoneLevelPercent(Number.NaN)).toBe(0);
    expect(microphonePeakPercent(-1)).toBe(0);
    expect(microphonePeakPercent(2)).toBe(100);
  });

  it('uses concise labels for stabilized backend classifications', () => {
    expect(microphoneClassificationLabel('no_signal')).toBe('No signal');
    expect(microphoneClassificationLabel('too_quiet')).toBe('Too quiet');
    expect(microphoneClassificationLabel('signal_detected')).toBe('Signal detected');
    expect(microphoneClassificationLabel('clipping')).toBe('Clipping');
  });
});
