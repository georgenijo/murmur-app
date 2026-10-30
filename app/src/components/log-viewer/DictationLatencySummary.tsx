import { useMemo } from 'react';
import type { PerformanceRunV1 } from '../../lib/performance';
import {
  dictationStagePercentiles,
  formatMilliseconds,
  type StagePercentiles,
} from '../../lib/performancePresentation';

interface DictationLatencySummaryProps {
  runs: PerformanceRunV1[];
}

function LatencyCard({
  label,
  scope,
  percentiles,
  emptyDetail,
}: {
  label: string;
  scope: string;
  percentiles: StagePercentiles | null;
  emptyDetail: string;
}) {
  return (
    <div role="group" aria-label={label} className="min-w-0 flex-1 rounded-xl border border-outline-variant/10 bg-surface-container-lowest p-3 shadow-sm">
      <div className="truncate text-[11px] font-medium text-on-surface-variant">{label}</div>
      {percentiles ? (
        <div className="mt-1 text-lg font-semibold tabular-nums text-on-surface">
          p50 {formatMilliseconds(percentiles.p50Ms)}
          <span className="text-on-surface-variant"> · </span>
          p90 {formatMilliseconds(percentiles.p90Ms)}
        </div>
      ) : (
        <div className="mt-1 text-lg font-semibold text-on-surface-variant">Unavailable</div>
      )}
      <div className="mt-1 text-[10px] leading-4 text-on-surface-variant">
        {percentiles
          ? `${scope} · ${percentiles.sampleCount} successful ${percentiles.sampleCount === 1 ? 'dictation' : 'dictations'}`
          : `${scope} · ${emptyDetail}`}
      </div>
    </div>
  );
}

/** Retained-run p50/p90 for post-stop processing and the full release-to-paste path. */
export function DictationLatencySummary({ runs }: DictationLatencySummaryProps) {
  const totalProcessing = useMemo(() => dictationStagePercentiles(runs, 'totalProcessing'), [runs]);
  const releaseToPaste = useMemo(() => dictationStagePercentiles(runs, 'releaseToPaste'), [runs]);
  return (
    <section aria-label="Dictation latency" className="mb-4 flex flex-wrap gap-3">
      <LatencyCard
        label="Total processing"
        scope="Stop accepted to paste"
        percentiles={totalProcessing}
        emptyDetail="No successful dictations yet"
      />
      <LatencyCard
        label="Release to paste"
        scope="Hotkey release to paste"
        percentiles={releaseToPaste}
        emptyDetail="No hold-to-talk release recorded yet"
      />
    </section>
  );
}
