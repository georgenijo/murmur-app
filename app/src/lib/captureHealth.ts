export const CAPTURE_HEALTH_WINDOW = 5;
export const SLOW_CAPTURE_STARTUP_MS = 2_000;

export type CaptureBackendName = 'auhal' | 'cpal';

export interface CaptureHealthObservationV1 {
  startupMs: number;
  usedFallback: boolean;
  fallbackFromBackend: CaptureBackendName | null;
}

export interface CaptureHealthHistoryV1 {
  schemaVersion: 1;
  observations: CaptureHealthObservationV1[];
}

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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function hasOwn(value: Record<string, unknown>, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function isCaptureBackend(value: unknown): value is CaptureBackendName {
  return value === 'auhal' || value === 'cpal';
}

function isObservation(value: unknown): value is CaptureHealthObservationV1 {
  return isRecord(value)
    && Object.keys(value).length === 3
    && hasOwn(value, 'startupMs')
    && hasOwn(value, 'usedFallback')
    && hasOwn(value, 'fallbackFromBackend')
    && typeof value.startupMs === 'number'
    && Number.isFinite(value.startupMs)
    && value.startupMs >= 0
    && typeof value.usedFallback === 'boolean'
    && (value.usedFallback || value.fallbackFromBackend === null)
    && (value.fallbackFromBackend === null || isCaptureBackend(value.fallbackFromBackend));
}

export function parseCaptureHealthHistory(value: unknown): CaptureHealthHistoryV1 {
  if (!isRecord(value)
    || Object.keys(value).length !== 2
    || !hasOwn(value, 'schemaVersion')
    || !hasOwn(value, 'observations')
    || value.schemaVersion !== 1
    || !Array.isArray(value.observations)
    || !value.observations.every(isObservation)) {
    throw new Error('Murmur returned an unsupported capture-health schema.');
  }
  return value as unknown as CaptureHealthHistoryV1;
}

function median(values: number[]): number {
  const sorted = [...values].sort((left, right) => left - right);
  const midpoint = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1
    ? sorted[midpoint]
    : (sorted[midpoint - 1] + sorted[midpoint]) / 2;
}

export function deriveCaptureHealth(observations: CaptureHealthObservationV1[]): CaptureHealth {
  const recent = observations.slice(-CAPTURE_HEALTH_WINDOW);
  const sampleCount = recent.length;
  const medianStartupMs = sampleCount > 0
    ? median(recent.map(observation => observation.startupMs))
    : null;
  const fallbacks = recent.filter(observation => observation.usedFallback);
  const fallbackCount = fallbacks.length;
  const enoughData = sampleCount === CAPTURE_HEALTH_WINDOW;
  const chronicFallback = enoughData && fallbackCount === CAPTURE_HEALTH_WINDOW;
  const slowStartup = enoughData
    && medianStartupMs !== null
    && medianStartupMs >= SLOW_CAPTURE_STARTUP_MS;
  const degradedBackends = fallbacks.map(observation => observation.fallbackFromBackend);
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
