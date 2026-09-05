import { useLayoutEffect, useSyncExternalStore } from 'react';
import packageMetadata from '../../package.json';

export const UI_LATENCY_SCHEMA_VERSION = 1 as const;
export const UI_LATENCY_SAMPLE_LIMIT = 500;
const STORAGE_KEY = 'murmur-ui-latency-v1';
const SERVER_SNAPSHOT: UiLatencySampleV1[] = [];

export type UiLatencyTrigger = 'pointer' | 'keyboard' | 'programmatic';

export interface UiLatencySampleV1 {
  schemaVersion: typeof UI_LATENCY_SCHEMA_VERSION;
  sampleId: string;
  from: string;
  to: string;
  trigger: UiLatencyTrigger;
  startedAtMs: number;
  commitMs: number;
  firstFrameMs?: number;
  frameIntervalMs?: number;
  paintedMs: number;
  build: string;
}

export interface UiLatencyEdgeSummary {
  from: string;
  to: string;
  count: number;
  medianCommitMs: number;
  medianFirstFrameMs: number | null;
  p95FirstFrameMs: number | null;
  medianFrameCount: number | null;
  medianPaintedMs: number;
  p95PaintedMs: number;
  maxPaintedMs: number;
}

interface PendingTransition {
  token: string;
  from: string;
  to: string;
  trigger: UiLatencyTrigger;
  startedAtMs: number;
  startedAtPerfMs: number;
  startMark: string;
}

const BUILD_REVISION = import.meta.env.VITE_MURMUR_BUILD_ID || 'unknown-revision';
const BUILD = `${packageMetadata.version}+${BUILD_REVISION} · ${import.meta.env.DEV ? 'development' : 'release'}`;
const listeners = new Set<() => void>();
const pendingByDestination = new Map<string, PendingTransition>();
let cachedSamples: UiLatencySampleV1[] | null = null;
let currentView = 'main.history';
let nextId = 1;
let storageListenerRegistered = false;
let pendingPersistence: number | null = null;
let pendingPersistenceUsesIdleCallback = false;

function isFiniteNonNegative(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0;
}

export function isUiLatencySampleV1(value: unknown): value is UiLatencySampleV1 {
  if (!value || typeof value !== 'object') return false;
  const sample = value as Partial<UiLatencySampleV1>;
  return sample.schemaVersion === UI_LATENCY_SCHEMA_VERSION
    && typeof sample.sampleId === 'string'
    && typeof sample.from === 'string'
    && typeof sample.to === 'string'
    && (sample.trigger === 'pointer' || sample.trigger === 'keyboard' || sample.trigger === 'programmatic')
    && isFiniteNonNegative(sample.startedAtMs)
    && isFiniteNonNegative(sample.commitMs)
    && (sample.firstFrameMs === undefined || isFiniteNonNegative(sample.firstFrameMs))
    && (sample.frameIntervalMs === undefined || isFiniteNonNegative(sample.frameIntervalMs))
    && isFiniteNonNegative(sample.paintedMs)
    && sample.paintedMs >= sample.commitMs
    && typeof sample.build === 'string';
}

function loadSamples(): UiLatencySampleV1[] {
  if (cachedSamples) return cachedSamples;
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]');
    cachedSamples = Array.isArray(parsed)
      ? parsed.filter(isUiLatencySampleV1).slice(-UI_LATENCY_SAMPLE_LIMIT)
      : [];
  } catch {
    cachedSamples = [];
  }
  return cachedSamples;
}

function persistSamples() {
  pendingPersistence = null;
  pendingPersistenceUsesIdleCallback = false;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(cachedSamples));
  } catch {
    // Timing remains available for this session when persistence is unavailable.
  }
}

function cancelPendingPersistence() {
  if (pendingPersistence === null || typeof window === 'undefined') return;
  if (pendingPersistenceUsesIdleCallback && typeof window.cancelIdleCallback === 'function') {
    window.cancelIdleCallback(pendingPersistence);
  } else {
    window.clearTimeout(pendingPersistence);
  }
  pendingPersistence = null;
  pendingPersistenceUsesIdleCallback = false;
}

function schedulePersistence() {
  if (typeof window === 'undefined') return;
  cancelPendingPersistence();
  if (typeof window.requestIdleCallback === 'function') {
    pendingPersistenceUsesIdleCallback = true;
    pendingPersistence = window.requestIdleCallback(persistSamples, { timeout: 750 });
  } else {
    pendingPersistence = window.setTimeout(persistSamples, 100);
  }
}

function publish(samples: UiLatencySampleV1[], deferPersistence = false) {
  cachedSamples = samples.slice(-UI_LATENCY_SAMPLE_LIMIT);
  if (deferPersistence) {
    schedulePersistence();
  } else {
    cancelPendingPersistence();
    persistSamples();
  }
  for (const listener of listeners) listener();
}

function appendSample(sample: UiLatencySampleV1) {
  publish([...loadSamples(), sample], true);
}

function sampleId(): string {
  const suffix = nextId++;
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return `${crypto.randomUUID()}-${suffix}`;
  }
  return `${Date.now()}-${suffix}`;
}

function scheduleFrame(callback: FrameRequestCallback): number {
  if (typeof requestAnimationFrame === 'function') {
    return requestAnimationFrame(callback);
  }
  return window.setTimeout(() => callback(performance.now()), 0);
}

function cancelFrame(frame: number) {
  if (typeof cancelAnimationFrame === 'function') {
    cancelAnimationFrame(frame);
  } else {
    window.clearTimeout(frame);
  }
}

function percentile(values: number[], percentileValue: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.max(0, Math.ceil(percentileValue * sorted.length) - 1);
  return sorted[index];
}

export function summarizeUiLatency(samples: UiLatencySampleV1[]): UiLatencyEdgeSummary[] {
  const groups = new Map<string, UiLatencySampleV1[]>();
  for (const sample of samples) {
    const key = `${sample.from}\u0000${sample.to}`;
    groups.set(key, [...(groups.get(key) ?? []), sample]);
  }
  return Array.from(groups.values())
    .map((edgeSamples) => {
      const firstFrames = edgeSamples.flatMap(sample =>
        sample.firstFrameMs === undefined ? [] : [sample.firstFrameMs]);
      const frameCounts = edgeSamples.flatMap(sample => {
        if (sample.firstFrameMs === undefined
          || sample.frameIntervalMs === undefined
          || sample.frameIntervalMs <= 0) return [];
        return [Math.max(0, Math.round(
          (sample.firstFrameMs - sample.commitMs) / sample.frameIntervalMs,
        ))];
      });
      return {
        from: edgeSamples[0].from,
        to: edgeSamples[0].to,
        count: edgeSamples.length,
        medianCommitMs: percentile(edgeSamples.map(sample => sample.commitMs), 0.5),
        medianFirstFrameMs: firstFrames.length > 0 ? percentile(firstFrames, 0.5) : null,
        p95FirstFrameMs: firstFrames.length > 0 ? percentile(firstFrames, 0.95) : null,
        medianFrameCount: frameCounts.length > 0 ? percentile(frameCounts, 0.5) : null,
        medianPaintedMs: percentile(edgeSamples.map(sample => sample.paintedMs), 0.5),
        p95PaintedMs: percentile(edgeSamples.map(sample => sample.paintedMs), 0.95),
        maxPaintedMs: Math.max(...edgeSamples.map(sample => sample.paintedMs)),
      };
    })
    .sort((left, right) =>
      (right.p95FirstFrameMs ?? right.p95PaintedMs)
      - (left.p95FirstFrameMs ?? left.p95PaintedMs)
      || left.from.localeCompare(right.from)
      || left.to.localeCompare(right.to));
}

export function getUiLatencyBuild(): string {
  return BUILD;
}

export function getUiLatencySamples(): UiLatencySampleV1[] {
  return loadSamples();
}

export function clearUiLatencySamples() {
  publish([]);
}

export function subscribeUiLatency(listener: () => void): () => void {
  if (!storageListenerRegistered && typeof window !== 'undefined') {
    storageListenerRegistered = true;
    window.addEventListener('storage', (event) => {
      if (event.key !== STORAGE_KEY) return;
      cancelPendingPersistence();
      cachedSamples = null;
      for (const subscriber of listeners) subscriber();
    });
  }
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useUiLatencySamples(): UiLatencySampleV1[] {
  return useSyncExternalStore(subscribeUiLatency, getUiLatencySamples, () => SERVER_SNAPSHOT);
}

export function beginUiTransition(
  from: string,
  to: string,
  trigger: UiLatencyTrigger,
): string | null {
  if (from === to) return null;

  const token = `${Date.now()}-${nextId++}`;
  const startMark = `ui-latency:${token}:start`;
  const transition: PendingTransition = {
    token,
    from,
    to,
    trigger,
    startedAtMs: Date.now(),
    startedAtPerfMs: performance.now(),
    startMark,
  };
  pendingByDestination.set(to, transition);
  performance.mark(startMark);
  return token;
}

export function beginCurrentUiTransition(
  to: string,
  trigger: UiLatencyTrigger,
): string | null {
  return beginUiTransition(currentView, to, trigger);
}

function commitUiTransition(destination: string): (() => void) | undefined {
  currentView = destination;
  const pending = pendingByDestination.get(destination);
  if (!pending) return undefined;

  const commitMs = Math.max(0, performance.now() - pending.startedAtPerfMs);
  let secondFrame = 0;
  let cancelled = false;
  const firstFrame = scheduleFrame((firstFrameAt) => {
    secondFrame = scheduleFrame((secondFrameAt) => {
      if (cancelled) return;
      if (pendingByDestination.get(destination)?.token !== pending.token) return;
      pendingByDestination.delete(destination);
      const firstFrameMs = Math.max(commitMs, firstFrameAt - pending.startedAtPerfMs);
      const frameIntervalMs = Math.max(0, secondFrameAt - firstFrameAt);
      const paintedMs = Math.max(commitMs, secondFrameAt - pending.startedAtPerfMs);
      const endMark = `ui-latency:${pending.token}:painted`;
      performance.mark(endMark);
      performance.measure(
        `UI ${pending.from} → ${pending.to}`,
        pending.startMark,
        endMark,
      );
      appendSample({
        schemaVersion: UI_LATENCY_SCHEMA_VERSION,
        sampleId: sampleId(),
        from: pending.from,
        to: pending.to,
        trigger: pending.trigger,
        startedAtMs: pending.startedAtMs,
        commitMs,
        firstFrameMs,
        frameIntervalMs,
        paintedMs,
        build: BUILD,
      });
    });
  });

  return () => {
    cancelled = true;
    cancelFrame(firstFrame);
    cancelFrame(secondFrame);
  };
}

/**
 * Completes a pending transition after React commits the destination and two
 * animation frames have allowed layout and paint to advance.
 */
export function useUiLatencyDestination(destination: string | null) {
  useLayoutEffect(() => {
    if (!destination) return undefined;
    return commitUiTransition(destination);
  }, [destination]);
}
