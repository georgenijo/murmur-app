import { useCallback, useEffect, useRef, useState } from 'react';
import {
  clearPerformanceDiagnostics,
  getPerformanceResourceWindow,
  listPerformanceRuns,
  onPerformanceDiagnosticsCleared,
  onPerformanceResourceSample,
  onPerformanceRunCompleted,
  type PerformanceRunV1,
  type ResourceSampleV1,
} from '../performance';

const MAX_RUNS = 200;
const MAX_SAMPLES = 600;

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

export function mergeRuns(
  current: PerformanceRunV1[],
  additions: PerformanceRunV1[],
): PerformanceRunV1[] {
  const byId = new Map(current.map(run => [run.runId, run]));
  for (const run of additions) byId.set(run.runId, run);
  return Array.from(byId.values())
    .sort((left, right) =>
      right.finishedAtMs - left.finishedAtMs || right.runId.localeCompare(left.runId))
    .slice(0, MAX_RUNS);
}

export function mergeResourceSamples(
  current: ResourceSampleV1[],
  additions: ResourceSampleV1[],
): ResourceSampleV1[] {
  const byTimestamp = new Map(current.map(sample => [sample.observedAtMs, sample]));
  for (const sample of additions) byTimestamp.set(sample.observedAtMs, sample);
  return Array.from(byTimestamp.values())
    .sort((left, right) => left.observedAtMs - right.observedAtMs)
    .slice(-MAX_SAMPLES);
}

interface InFlightRead {
  generation: number;
  promise: Promise<void>;
}

export function usePerformanceDiagnostics(enabled: boolean) {
  const [runs, setRuns] = useState<PerformanceRunV1[]>([]);
  const [samples, setSamples] = useState<ResourceSampleV1[]>([]);
  const [runsLoading, setRunsLoading] = useState(true);
  const [resourcesLoading, setResourcesLoading] = useState(true);
  const [runsError, setRunsError] = useState<string | null>(null);
  const [resourcesError, setResourcesError] = useState<string | null>(null);
  const [clearError, setClearError] = useState<string | null>(null);
  const [cleared, setCleared] = useState(false);
  const [clearing, setClearing] = useState(false);
  const dataGenerationRef = useRef(0);
  // One read per source at a time. A second request for the same data
  // generation (React remounting an effect in development, or a retry while a
  // load is pending) shares the pending read instead of querying SQLite again.
  const runsInFlightRef = useRef<InFlightRead | null>(null);
  const resourcesInFlightRef = useRef<InFlightRead | null>(null);

  const refreshRuns = useCallback(() => {
    const pending = runsInFlightRef.current;
    if (pending && pending.generation === dataGenerationRef.current) return pending.promise;
    const generation = dataGenerationRef.current;
    const entry = { generation } as InFlightRead;
    runsInFlightRef.current = entry;
    entry.promise = (async () => {
      setRunsLoading(true);
      setRunsError(null);
      try {
        const response = await listPerformanceRuns(MAX_RUNS);
        if (generation === dataGenerationRef.current) {
          setRuns(current => mergeRuns(response.runs, current));
          if (response.runs.length > 0) setCleared(false);
        }
      } catch (reason) {
        setRunsError(errorMessage(reason));
      } finally {
        setRunsLoading(false);
        if (runsInFlightRef.current === entry) runsInFlightRef.current = null;
      }
    })();
    return entry.promise;
  }, []);

  const refreshResources = useCallback(() => {
    const pending = resourcesInFlightRef.current;
    if (pending && pending.generation === dataGenerationRef.current) return pending.promise;
    const generation = dataGenerationRef.current;
    const entry = { generation } as InFlightRead;
    resourcesInFlightRef.current = entry;
    entry.promise = (async () => {
      setResourcesLoading(true);
      setResourcesError(null);
      try {
        const response = await getPerformanceResourceWindow();
        if (generation === dataGenerationRef.current) {
          setSamples(current => mergeResourceSamples(response, current));
        }
      } catch (reason) {
        setResourcesError(errorMessage(reason));
      } finally {
        setResourcesLoading(false);
        if (resourcesInFlightRef.current === entry) resourcesInFlightRef.current = null;
      }
    })();
    return entry.promise;
  }, []);

  const refresh = useCallback(async () => {
    await Promise.all([refreshRuns(), refreshResources()]);
  }, [refreshResources, refreshRuns]);

  const clear = useCallback(async () => {
    dataGenerationRef.current += 1;
    setClearing(true);
    setClearError(null);
    try {
      await clearPerformanceDiagnostics();
      setRuns([]);
      setSamples([]);
      setRunsLoading(false);
      setResourcesLoading(false);
      setCleared(true);
    } catch (reason) {
      setClearError(errorMessage(reason));
      throw reason;
    } finally {
      setClearing(false);
    }
  }, []);

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    const unlisteners: Array<Promise<() => void>> = [];

    unlisteners.push(onPerformanceRunCompleted(run => {
      if (disposed) return;
      setRuns(current => mergeRuns(current, [run]));
      setRunsLoading(false);
      setRunsError(null);
      setCleared(false);
    }));
    unlisteners.push(onPerformanceResourceSample(sample => {
      if (disposed) return;
      setSamples(current => mergeResourceSamples(current, [sample]));
      setResourcesLoading(false);
      setResourcesError(null);
    }));
    unlisteners.push(onPerformanceDiagnosticsCleared(() => {
      if (disposed) return;
      dataGenerationRef.current += 1;
      setRuns([]);
      setSamples([]);
      setRunsLoading(false);
      setResourcesLoading(false);
      setCleared(true);
    }));

    void refresh();
    return () => {
      disposed = true;
      for (const unlisten of unlisteners) void unlisten.then(stop => stop());
    };
  }, [enabled, refresh]);

  return {
    runs,
    samples,
    runsLoading,
    resourcesLoading,
    runsError,
    resourcesError,
    clearError,
    cleared,
    clearing,
    refreshRuns,
    refreshResources,
    refresh,
    clear,
  };
}
