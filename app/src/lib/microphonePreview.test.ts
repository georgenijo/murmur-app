import { describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import {
  microphoneClassificationLabel,
  microphoneLevelPercent,
  microphonePeakPercent,
  startMicrophonePreview,
  smoothMicrophoneMeterValue,
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

  it('smooths meter movement with a faster attack than release', () => {
    const attack = smoothMicrophoneMeterValue(0, 80, 16);
    const release = smoothMicrophoneMeterValue(80, 0, 16);

    expect(attack).toBeGreaterThan(0);
    expect(attack).toBeLessThan(80);
    expect(80 - release).toBeLessThan(attack);
    expect(smoothMicrophoneMeterValue(20, 20, 16)).toBe(20);
  });

  it('bounds malformed meter animation inputs', () => {
    expect(smoothMicrophoneMeterValue(Number.NaN, 200, 16)).toBeGreaterThan(0);
    expect(smoothMicrophoneMeterValue(50, Number.NaN, Number.NaN)).toBe(50);
    expect(smoothMicrophoneMeterValue(-20, 0, 16)).toBe(0);
  });

  it('lets the Smart Auto request own preview selection instead of sending a fixed device too', async () => {
    invoke.mockResolvedValue({});
    const smartAuto = {
      approvedDeviceIds: ['usb'],
      preferredDeviceIds: ['usb'],
      allowContinuity: false,
      requireRecentSignal: false,
    };

    await startMicrophonePreview('usb', 60, smartAuto);

    expect(invoke).toHaveBeenCalledWith('start_microphone_preview', {
      deviceId: 'system_default',
      vadSensitivity: 60,
      smartAuto,
    });
  });
});
