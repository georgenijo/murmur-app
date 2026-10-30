import { measuredValue, type PerformanceRunV1 } from './performance';

/** Content-free cold-load rate over the bounded local run history (#831). */
export interface ColdModelLoadSummary {
  /** Successful dictations whose transcription warm state is known. */
  dictations: number;
  coldLoaded: number;
  /** Median measured model load across cold-loaded runs, if any. */
  medianColdLoadMs: number | null;
}

function median(values: number[]): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[middle - 1] + sorted[middle]) / 2
    : sorted[middle];
}

export function summarizeColdModelLoads(runs: readonly PerformanceRunV1[]): ColdModelLoadSummary {
  let dictations = 0;
  let coldLoaded = 0;
  const loadMs: number[] = [];
  for (const run of runs) {
    if (run.kind !== 'dictation' || run.outcome.status !== 'success') continue;
    const runtime = run.runtimes.find(entry => entry.role === 'transcription');
    if (!runtime || runtime.warmState === 'unknown') continue;
    dictations += 1;
    if (runtime.warmState !== 'coldLoaded') continue;
    coldLoaded += 1;
    const stage = run.stages.find(entry => entry.stage === 'modelLoad');
    const value = stage ? measuredValue(stage.durationMs) : null;
    if (value !== null) loadMs.push(value);
  }
  return { dictations, coldLoaded, medianColdLoadMs: median(loadMs) };
}
