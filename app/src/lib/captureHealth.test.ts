import { describe, expect, it } from 'vitest';
import {
  CAPTURE_HEALTH_WINDOW,
  deriveCaptureHealth,
  parseCaptureHealthHistory,
  SLOW_CAPTURE_STARTUP_MS,
  type CaptureHealthObservationV1,
} from './captureHealth';

function observation(
  startupMs: number,
  fallbackFromBackend: CaptureHealthObservationV1['fallbackFromBackend'] = null,
): CaptureHealthObservationV1 {
  return {
    startupMs,
    usedFallback: fallbackFromBackend !== null,
    fallbackFromBackend,
  };
}

describe('deriveCaptureHealth', () => {
  it('requires five successful dictation captures before judging health', () => {
    const health = deriveCaptureHealth([
      observation(180),
      observation(220),
      observation(240),
      observation(260),
    ]);

    expect(health.status).toBe('insufficientData');
    expect(health.sampleCount).toBe(CAPTURE_HEALTH_WINDOW - 1);
    expect(health.medianStartupMs).toBe(230);
  });

  it('reports a healthy rolling median and occasional fallback honestly', () => {
    const health = deriveCaptureHealth([
      observation(180),
      observation(220),
      observation(320, 'auhal'),
      observation(240),
      observation(260),
    ]);

    expect(health.status).toBe('healthy');
    expect(health.medianStartupMs).toBe(240);
    expect(health.fallbackCount).toBe(1);
    expect(health.chronicFallback).toBe(false);
  });

  it('preserves the verified five-of-five recovered-fallback threshold', () => {
    const health = deriveCaptureHealth(
      Array.from({ length: CAPTURE_HEALTH_WINDOW }, (_, index) => observation(700 + index, 'auhal')),
    );

    expect(health.status).toBe('degraded');
    expect(health.chronicFallback).toBe(true);
    expect(health.slowStartup).toBe(false);
    expect(health.degradedBackend).toBe('auhal');
  });

  it('does not call four of five fallbacks chronic', () => {
    const health = deriveCaptureHealth([
      observation(700, 'auhal'),
      observation(701, 'auhal'),
      observation(702, 'auhal'),
      observation(703, 'auhal'),
      observation(200),
    ]);

    expect(health.status).toBe('healthy');
    expect(health.fallbackCount).toBe(4);
    expect(health.chronicFallback).toBe(false);
  });

  it('flags a slow rolling median without inventing a degraded backend', () => {
    const health = deriveCaptureHealth([
      observation(SLOW_CAPTURE_STARTUP_MS - 1),
      observation(SLOW_CAPTURE_STARTUP_MS),
      observation(2_500),
      observation(2_800),
      observation(3_100),
    ]);

    expect(health.status).toBe('degraded');
    expect(health.slowStartup).toBe(true);
    expect(health.chronicFallback).toBe(false);
    expect(health.degradedBackend).toBeNull();
  });
});

describe('parseCaptureHealthHistory', () => {
  it('accepts only the versioned content-free observation shape', () => {
    expect(parseCaptureHealthHistory({
      schemaVersion: 1,
      observations: [{ startupMs: 240, usedFallback: true, fallbackFromBackend: 'cpal' }],
    }).observations).toHaveLength(1);

    expect(() => parseCaptureHealthHistory({
      schemaVersion: 1,
      observations: [{ startupMs: 240, usedFallback: true, fallbackFromBackend: 'device label' }],
    })).toThrow(/unsupported capture-health schema/i);
    expect(() => parseCaptureHealthHistory({
      schemaVersion: 1,
      observations: [{
        startupMs: 240,
        usedFallback: false,
        fallbackFromBackend: null,
        deviceUid: 'private',
      }],
    })).toThrow(/unsupported capture-health schema/i);

    expect(() => parseCaptureHealthHistory({
      schemaVersion: 1,
      observations: [{ startupMs: 240, usedFallback: false, fallbackFromBackend: 'auhal' }],
    })).toThrow(/unsupported capture-health schema/i);
  });
});
