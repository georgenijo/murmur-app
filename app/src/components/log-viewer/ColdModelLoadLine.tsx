import { useMemo } from 'react';
import { summarizeColdModelLoads } from '../../lib/coldModelLoads';
import type { PerformanceRunV1 } from '../../lib/performance';
import { formatMilliseconds } from '../../lib/performancePresentation';

/** Cold-load rate across the bounded local run history (issue #831). */
export function ColdModelLoadLine({ runs }: { runs: readonly PerformanceRunV1[] }) {
  const summary = useMemo(() => summarizeColdModelLoads(runs), [runs]);
  if (summary.dictations === 0) {
    return (
      <div role="status" aria-label="Cold model loads" className="mb-3 rounded-xl border border-outline-variant/15 bg-surface-container-low px-3 py-2">
        <div className="text-xs font-medium text-on-surface">Cold model loads: no recent dictations yet</div>
      </div>
    );
  }
  const percent = Math.round((summary.coldLoaded / summary.dictations) * 100);
  return (
    <div role="status" aria-label="Cold model loads" className="mb-3 rounded-xl border border-outline-variant/15 bg-surface-container-low px-3 py-2">
      <div className="text-xs font-medium text-on-surface">
        Cold model loads: {summary.coldLoaded} of {summary.dictations} recent {summary.dictations === 1 ? 'dictation' : 'dictations'} ({percent}%)
      </div>
      <p className="mt-0.5 text-[11px] text-on-surface-variant">
        {summary.medianColdLoadMs !== null && `Median cold load ${formatMilliseconds(summary.medianColdLoadMs)}. `}
        Idle model release can cause cold loads; Speech-to-text → Release model after inactivity controls idle release.
      </p>
    </div>
  );
}
