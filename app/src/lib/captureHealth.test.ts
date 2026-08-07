import { describe, expect, it } from 'vitest';
import type { AppEvent } from './events';
import {
  CAPTURE_HEALTH_WINDOW,
  deriveCaptureHealth,
  SLOW_CAPTURE_STARTUP_MS,
} from './captureHealth';

function event(
  seconds: number,
  summary: string,
  data: Record<string, unknown>,
): AppEvent {
  return {
    timestamp: new Date(Date.UTC(2026, 7, 7, 12, 0, seconds)).toISOString(),
    stream: 'audio',
    level: 'info',
    summary,
    data,
  };
}

function ready(seconds: number, owner: number, startupMs: number): AppEvent {
  return event(seconds, 'audio readiness accepted', {
    event_code: 'audio.capture_ready',
    owner,
    owner_kind: 'dictation',
    startup_ms: startupMs,
  });
}

function fallback(seconds: number, owner: number, fromBackend = 'auhal'): AppEvent {
  return event(seconds, 'capture backend failed before retained audio; trying bounded fallback', {
    event_code: 'audio.fallback_started',
    owner,
    from_backend: fromBackend,
    to_backend: 'cpal',
  });
}

describe('deriveCaptureHealth', () => {
  it('requires five successful dictation captures before judging health', () => {
    const health = deriveCaptureHealth([
      ready(1, 1, 180),
      ready(2, 2, 220),
      ready(3, 3, 240),
      ready(4, 4, 260),
    ]);

    expect(health.status).toBe('insufficientData');
    expect(health.sampleCount).toBe(CAPTURE_HEALTH_WINDOW - 1);
    expect(health.medianStartupMs).toBe(230);
  });

  it('reports a healthy rolling median and occasional fallback honestly', () => {
    const health = deriveCaptureHealth([
      ready(1, 1, 180),
      ready(2, 2, 220),
      fallback(3, 3),
      ready(4, 3, 320),
      ready(5, 4, 240),
      ready(6, 5, 260),
    ]);

    expect(health.status).toBe('healthy');
    expect(health.medianStartupMs).toBe(240);
    expect(health.fallbackCount).toBe(1);
    expect(health.chronicFallback).toBe(false);
  });

  it('flags five consecutive recovered fallbacks and names the failed backend', () => {
    const events: AppEvent[] = [];
    for (let owner = 1; owner <= CAPTURE_HEALTH_WINDOW; owner += 1) {
      events.push(fallback(owner * 2, owner));
      events.push(ready(owner * 2 + 1, owner, 700 + owner));
    }

    const health = deriveCaptureHealth(events);
    expect(health.status).toBe('degraded');
    expect(health.chronicFallback).toBe(true);
    expect(health.slowStartup).toBe(false);
    expect(health.degradedBackend).toBe('auhal');
  });

  it('flags a slow rolling median without inventing a degraded backend', () => {
    const health = deriveCaptureHealth([
      ready(1, 1, SLOW_CAPTURE_STARTUP_MS - 1),
      ready(2, 2, SLOW_CAPTURE_STARTUP_MS),
      ready(3, 3, 2_500),
      ready(4, 4, 2_800),
      ready(5, 5, 3_100),
    ]);

    expect(health.status).toBe('degraded');
    expect(health.slowStartup).toBe(true);
    expect(health.chronicFallback).toBe(false);
    expect(health.degradedBackend).toBeNull();
  });

  it('does not correlate a stale fallback across a fresh owner start', () => {
    const health = deriveCaptureHealth([
      fallback(1, 1),
      event(2, 'audio initialization accepted', { owner: 1 }),
      ready(3, 1, 200),
      ready(4, 2, 210),
      ready(5, 3, 220),
      ready(6, 4, 230),
      ready(7, 5, 240),
    ]);

    expect(health.status).toBe('healthy');
    expect(health.fallbackCount).toBe(0);
  });

  it('accepts exact historical summaries when stable event codes are absent', () => {
    const events: AppEvent[] = [];
    for (let owner = 1; owner <= CAPTURE_HEALTH_WINDOW; owner += 1) {
      events.push(event(owner * 2, 'capture backend failed before retained audio; trying bounded fallback', {
        owner,
        from_backend: 'auhal',
      }));
      events.push(event(owner * 2 + 1, 'audio readiness accepted', {
        owner,
        owner_kind: 'dictation',
        startup_ms: 650,
      }));
    }

    expect(deriveCaptureHealth(events).chronicFallback).toBe(true);
  });
});
