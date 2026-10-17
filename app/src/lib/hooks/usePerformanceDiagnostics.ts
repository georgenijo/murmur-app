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
  const activeRef = useRef(false);
  const runsRequestRef = useRef(0);
  const resourcesRequestRef = useRef(0);
  const liveRunsRef = useRef<PerformanceRunV1[] | null>(null);
  const liveSamplesRef = useRef<ResourceSampleV1[] | null>(null);

  const refreshRuns = useCallback(async () => {
    if (!activeRef.current) return;
    const generation = dataGenerationRef.current;
    const request = ++runsRequestRef.current;
    liveRunsRef.current = [];
    const currentRequest = () => activeRef.current
      && generation === dataGenerationRef.current && request === runsRequestRef.current;
    setRunsLoading(true);
    setRunsError(null);
    try {
      const response = await listPerformanceRuns(MAX_RUNS);
      if (currentRequest()) {
        // The backend snapshot is authoritative after a hidden interval (it
        // may have been cleared elsewhere). Retain only live arrivals since
        // this read began, rather than resurrecting the old cached snapshot.
        setRuns(mergeRuns(response.runs, liveRunsRef.current ?? []));
        if (response.runs.length > 0) setCleared(false);
      }
    } catch (reason) {
      if (currentRequest()) setRunsError(errorMessage(reason));
    } finally {
      if (currentRequest()) {
        liveRunsRef.current = null;
        setRunsLoading(false);
      }
    }
  }, []);

  const refreshResources = useCallback(async () => {
    if (!activeRef.current) return;
    const generation = dataGenerationRef.current;
    const request = ++resourcesRequestRef.current;
    liveSamplesRef.current = [];
    const currentRequest = () => activeRef.current
      && generation === dataGenerationRef.current && request === resourcesRequestRef.current;
    setResourcesLoading(true);
    setResourcesError(null);
    try {
      const response = await getPerformanceResourceWindow();
      if (currentRequest()) {
        setSamples(mergeResourceSamples(response, liveSamplesRef.current ?? []));
      }
    } catch (reason) {
      if (currentRequest()) setResourcesError(errorMessage(reason));
    } finally {
      if (currentRequest()) {
        liveSamplesRef.current = null;
        setResourcesLoading(false);
      }
    }
  }, []);

  const refresh = useCallback(async () => {
    await Promise.all([refreshRuns(), refreshResources()]);
  }, [refreshResources, refreshRuns]);

  const clear = useCallback(async () => {
    dataGenerationRef.current += 1;
    liveRunsRef.current = null;
    liveSamplesRef.current = null;
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
    activeRef.current = true;
    let disposed = false;
    const unlisteners: Array<Promise<() => void>> = [];

    unlisteners.push(onPerformanceRunCompleted(run => {
      if (disposed) return;
      if (liveRunsRef.current !== null) {
        liveRunsRef.current = mergeRuns(liveRunsRef.current, [run]);
      }
      setRuns(current => mergeRuns(current, [run]));
      setRunsLoading(false);
      setRunsError(null);
      setCleared(false);
    }));
    unlisteners.push(onPerformanceResourceSample(sample => {
      if (disposed) return;
      if (liveSamplesRef.current !== null) {
        liveSamplesRef.current = mergeResourceSamples(liveSamplesRef.current, [sample]);
      }
      setSamples(current => mergeResourceSamples(current, [sample]));
      setResourcesLoading(false);
      setResourcesError(null);
    }));
    unlisteners.push(onPerformanceDiagnosticsCleared(() => {
      if (disposed) return;
      dataGenerationRef.current += 1;
      liveRunsRef.current = null;
      liveSamplesRef.current = null;
      setRuns([]);
      setSamples([]);
      setRunsLoading(false);
      setResourcesLoading(false);
      setCleared(true);
    }));

    void refresh();
    return () => {
      disposed = true;
      activeRef.current = false;
      dataGenerationRef.current += 1;
      liveRunsRef.current = null;
      liveSamplesRef.current = null;
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
