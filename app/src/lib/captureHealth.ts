import type { AppEvent } from './events';

export const CAPTURE_HEALTH_WINDOW = 5;
export const SLOW_CAPTURE_STARTUP_MS = 2_000;

const FALLBACK_CORRELATION_WINDOW_MS = 35_000;
const READY_SUMMARY = 'audio readiness accepted';
const FALLBACK_SUMMARY = 'capture backend failed before retained audio; trying bounded fallback';
const START_SUMMARY = 'audio initialization accepted';

export type CaptureBackendName = 'auhal' | 'cpal';

export interface CaptureHealth {
  status: 'insufficientData' | 'healthy' | 'degraded';
  sampleCount: number;
  requiredSamples: number;
  medianStartupMs: number | null;
  fallbackCount: number;
  chronicFallback: boolean;
  slowStartup: boolean;
  degradedBackend: CaptureBackendName | null;
}

interface PendingFallback {
  atMs: number;
  fromBackend: CaptureBackendName | null;
}

interface CaptureObservation {
  startupMs: number;
  fallback: PendingFallback | null;
}

function eventCode(event: AppEvent): string | null {
  return typeof event.data.event_code === 'string' ? event.data.event_code : null;
}

function numericField(event: AppEvent, key: string): number | null {
  const value = event.data[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function backendField(event: AppEvent, key: string): CaptureBackendName | null {
  const value = event.data[key];
  return value === 'auhal' || value === 'cpal' ? value : null;
}

function observedAtMs(event: AppEvent): number | null {
  const value = Date.parse(event.timestamp);
  return Number.isFinite(value) ? value : null;
}

function isReady(event: AppEvent): boolean {
  return eventCode(event) === 'audio.capture_ready' || event.summary === READY_SUMMARY;
}

function isFallback(event: AppEvent): boolean {
  return eventCode(event) === 'audio.fallback_started' || event.summary === FALLBACK_SUMMARY;
}

function isCaptureFailed(event: AppEvent): boolean {
  return eventCode(event) === 'audio.capture_failed';
}

function median(values: number[]): number {
  const sorted = [...values].sort((left, right) => left - right);
  const midpoint = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1
    ? sorted[midpoint]
    : (sorted[midpoint - 1] + sorted[midpoint]) / 2;
}

export function deriveCaptureHealth(events: AppEvent[]): CaptureHealth {
  const pendingFallbacks = new Map<number, PendingFallback>();
  const observations: CaptureObservation[] = [];

  for (const event of events) {
    const owner = numericField(event, 'owner');
    if (owner === null) continue;

    if (event.summary === START_SUMMARY) {
      pendingFallbacks.delete(owner);
      continue;
    }
    if (isFallback(event)) {
      const atMs = observedAtMs(event);
      if (atMs !== null) {
        pendingFallbacks.set(owner, {
          atMs,
          fromBackend: backendField(event, 'from_backend'),
        });
      }
      continue;
    }
    if (isCaptureFailed(event)) {
      pendingFallbacks.delete(owner);
      continue;
    }
    if (!isReady(event) || event.data.owner_kind !== 'dictation') continue;

    const startupMs = numericField(event, 'startup_ms');
    const readyAtMs = observedAtMs(event);
    if (startupMs === null || startupMs < 0 || readyAtMs === null) continue;

    const pending = pendingFallbacks.get(owner) ?? null;
    const fallback = pending
      && readyAtMs >= pending.atMs
      && readyAtMs - pending.atMs <= FALLBACK_CORRELATION_WINDOW_MS
      ? pending
      : null;
    observations.push({ startupMs, fallback });
    pendingFallbacks.delete(owner);
  }

  const recent = observations.slice(-CAPTURE_HEALTH_WINDOW);
  const sampleCount = recent.length;
  const medianStartupMs = sampleCount > 0
    ? median(recent.map(observation => observation.startupMs))
    : null;
  const fallbacks = recent.filter(observation => observation.fallback !== null);
  const fallbackCount = fallbacks.length;
  const enoughData = sampleCount === CAPTURE_HEALTH_WINDOW;
  const chronicFallback = enoughData && fallbackCount === CAPTURE_HEALTH_WINDOW;
  const slowStartup = enoughData
    && medianStartupMs !== null
    && medianStartupMs >= SLOW_CAPTURE_STARTUP_MS;
  const degradedBackends = fallbacks.map(observation => observation.fallback?.fromBackend ?? null);
  const degradedBackend = chronicFallback
    && degradedBackends[0] !== null
    && degradedBackends.every(backend => backend === degradedBackends[0])
    ? degradedBackends[0]
    : null;

  return {
    status: !enoughData
      ? 'insufficientData'
      : chronicFallback || slowStartup
        ? 'degraded'
        : 'healthy',
    sampleCount,
    requiredSamples: CAPTURE_HEALTH_WINDOW,
    medianStartupMs,
    fallbackCount,
    chronicFallback,
    slowStartup,
    degradedBackend,
  };
}
