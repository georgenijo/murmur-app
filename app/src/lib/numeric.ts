/**
 * Small numeric helpers reused by latency/benchmark summarizers. Both were
 * previously re-declared identically (percentile: `uiLatency.ts` and
 * `LatencyMapView.tsx`; median: `productionComparison.ts` and
 * `captureHealth.ts`) — see F12a cleanup context.
 */

/**
 * Nearest-rank percentile over a copy of `values` (does not mutate the
 * input). Returns 0 for an empty input, matching both prior call sites,
 * which never rendered a percentile without first checking `length > 0`.
 */
export function percentile(values: readonly number[], fraction: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.max(0, Math.ceil(fraction * sorted.length) - 1)];
}

/**
 * Median over a copy of `values` (does not mutate the input). Returns null
 * for an empty input rather than `NaN`.
 */
export function median(values: readonly number[]): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[middle - 1] + sorted[middle]) / 2
    : sorted[middle];
}
