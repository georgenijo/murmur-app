import { useMemo, useState } from 'react';
import type { PerformanceRunV1 } from '../../lib/performance';
import {
  compareProductionVersions,
  productionVersionOptions,
  type ProductionMetricComparison,
  type StatisticComparison,
  type VersionObservation,
} from '../../lib/productionComparison';

interface ProductionComparisonProps {
  runs: readonly PerformanceRunV1[];
}

function formatMilliseconds(value: number | null): string {
  if (value === null) return 'Unavailable';
  return `${value.toLocaleString(undefined, { maximumFractionDigits: 1 })} ms`;
}

function formatDelta(comparison: StatisticComparison): string {
  if (comparison.absoluteDeltaMs === null) return 'Unavailable';
  const sign = comparison.absoluteDeltaMs > 0 ? '+' : '';
  const percentage = comparison.percentageDelta === null
    ? 'relative change unavailable'
    : `${comparison.percentageDelta > 0 ? '+' : ''}${comparison.percentageDelta.toFixed(1)}%`;
  return `${sign}${comparison.absoluteDeltaMs.toLocaleString(undefined, { maximumFractionDigits: 1 })} ms · ${percentage}`;
}

function rawValues(metric: ProductionMetricComparison): string {
  const values = (side: readonly number[]) => side.length > 0
    ? side.map(value => `${value} ms`).join(', ')
    : 'none measured';
  return `Baseline: ${values(metric.baseline.rawMs)}. Candidate: ${values(metric.candidate.rawMs)}.`;
}

function observationEvidence(observation: VersionObservation): string {
  if (observation.key === 'success') return 'Version-wide outcome count';
  const finding = observation.newlyObserved
    ? 'New in candidate'
    : observation.candidateObserved
      ? 'Observed in candidate'
      : 'No candidate anomaly observed';
  const unavailable = observation.baselineUnavailable + observation.candidateUnavailable > 0
    ? `${observation.baselineUnavailable} baseline and ${observation.candidateUnavailable} candidate unavailable`
    : null;
  return unavailable ? `${finding} · ${unavailable}` : finding;
}

function MetricRow({ metric }: { metric: ProductionMetricComparison }) {
  const regression = metric.median.verdict === 'regression' || metric.p95.verdict === 'regression';
  return (
    <>
      <tr className={`border-t border-outline-variant/10 ${regression ? 'bg-warning/10' : ''}`}>
        <th scope="row" className="px-3 py-2 font-medium text-on-surface">
          {metric.definition.label}
          {regression && (
            <span className="ml-2 rounded-full bg-warning/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-warning">
              Regression
            </span>
          )}
        </th>
        <td className="whitespace-nowrap px-3 py-2 text-right tabular-nums text-on-surface">
          {formatMilliseconds(metric.baseline.medianMs)}
          <span className="block text-[9px] text-on-surface-variant">
            {metric.baseline.measuredCount} measured, {metric.baseline.missingCount} unavailable
          </span>
        </td>
        <td className="whitespace-nowrap px-3 py-2 text-right tabular-nums text-on-surface">
          {formatMilliseconds(metric.candidate.medianMs)}
          <span className="block text-[9px] text-on-surface-variant">
            {metric.candidate.measuredCount} measured, {metric.candidate.missingCount} unavailable
          </span>
        </td>
        <td className={`whitespace-nowrap px-3 py-2 text-right tabular-nums ${metric.median.verdict === 'regression' ? 'font-semibold text-warning' : 'text-on-surface'}`}>
          {formatDelta(metric.median)}
        </td>
        <td className="whitespace-nowrap px-3 py-2 text-right tabular-nums text-on-surface">
          {metric.p95.baselineMs === null || metric.p95.candidateMs === null
            ? 'Insufficient samples'
            : `${formatMilliseconds(metric.p95.baselineMs)} → ${formatMilliseconds(metric.p95.candidateMs)}`}
          {metric.p95.absoluteDeltaMs !== null && (
            <span className={`block text-[9px] ${metric.p95.verdict === 'regression' ? 'font-semibold text-warning' : 'text-on-surface-variant'}`}>
              {formatDelta(metric.p95)}
            </span>
          )}
        </td>
      </tr>
      {metric.preliminary && (
        <tr className="border-t border-outline-variant/5 bg-surface-container-low/40">
          <td className="px-3 pb-2 pt-1 text-[9px] font-semibold uppercase tracking-wide text-on-surface-variant">
            Preliminary raw values
          </td>
          <td colSpan={4} className="px-3 pb-2 pt-1 text-[10px] tabular-nums text-on-surface-variant">
            {rawValues(metric)}
          </td>
        </tr>
      )}
    </>
  );
}

export function ProductionComparison({ runs }: ProductionComparisonProps) {
  const versions = useMemo(() => productionVersionOptions(runs), [runs]);
  const [chosenBaseline, setChosenBaseline] = useState('');
  const [chosenCandidate, setChosenCandidate] = useState('');
  const candidateVersion = versions.some(option => option.appVersion === chosenCandidate)
    ? chosenCandidate
    : versions[0]?.appVersion ?? '';
  const baselineVersion = versions.some(option =>
    option.appVersion === chosenBaseline && option.appVersion !== candidateVersion)
    ? chosenBaseline
    : versions.find(option => option.appVersion !== candidateVersion)?.appVersion ?? '';
  const comparison = useMemo(() => {
    if (!baselineVersion || !candidateVersion || baselineVersion === candidateVersion) return null;
    return compareProductionVersions(runs, baselineVersion, candidateVersion);
  }, [baselineVersion, candidateVersion, runs]);

  return (
    <details className="mb-4 rounded-xl border border-outline-variant/15 bg-surface-container-lowest shadow-sm">
      <summary className="cursor-pointer select-none px-3 py-2.5 text-xs font-semibold text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">
        Compare app versions
      </summary>
      <div className="border-t border-outline-variant/10 p-3">
        <p className="text-[11px] leading-4 text-on-surface-variant">
          Murmur compares successful dictation measurements only within exact release cohorts. Version-wide failures and capture anomalies stay visible even when latency cannot be compared.
        </p>

        {versions.length < 2 ? (
          <p className="mt-3 rounded-lg border border-dashed border-outline-variant/30 bg-surface-container-low px-3 py-3 text-xs text-on-surface-variant">
            Dictation runs from two app versions are needed before a comparison can be made.
          </p>
        ) : comparison && (
          <>
            <div className="mt-3 flex flex-wrap gap-3">
              <label className="text-[10px] font-medium uppercase tracking-wider text-on-surface-variant">
                Baseline
                <select
                  aria-label="Baseline app version"
                  value={baselineVersion}
                  onChange={event => setChosenBaseline(event.target.value)}
                  className="mt-1 block rounded-lg border border-on-surface-variant bg-surface-container-lowest px-2 py-1.5 text-xs normal-case tracking-normal text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
                >
                  {versions.filter(option => option.appVersion !== candidateVersion).map(option => (
                    <option key={option.appVersion} value={option.appVersion}>
                      {option.appVersion} · {option.runCount} dictation {option.runCount === 1 ? 'run' : 'runs'}
                    </option>
                  ))}
                </select>
              </label>
              <label className="text-[10px] font-medium uppercase tracking-wider text-on-surface-variant">
                Candidate
                <select
                  aria-label="Candidate app version"
                  value={candidateVersion}
                  onChange={event => setChosenCandidate(event.target.value)}
                  className="mt-1 block rounded-lg border border-on-surface-variant bg-surface-container-lowest px-2 py-1.5 text-xs normal-case tracking-normal text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
                >
                  {versions.filter(option => option.appVersion !== baselineVersion).map(option => (
                    <option key={option.appVersion} value={option.appVersion}>
                      {option.appVersion} · {option.runCount} dictation {option.runCount === 1 ? 'run' : 'runs'}
                    </option>
                  ))}
                </select>
              </label>
            </div>

            <section aria-labelledby="version-observations-heading" className="mt-4">
              <div className="flex flex-wrap items-end justify-between gap-2">
                <div>
                  <h3 id="version-observations-heading" className="text-xs font-semibold text-on-surface">
                    Version-wide observations
                  </h3>
                  <p className="text-[10px] text-on-surface-variant">
                    Counts cover every dictation run for the selected versions. They do not claim a compatible latency cause.
                  </p>
                </div>
                <span className="text-[10px] tabular-nums text-on-surface-variant">
                  {baselineVersion}: {comparison.baselineDictationCount} · {candidateVersion}: {comparison.candidateDictationCount}
                </span>
              </div>
              <div className="mt-2 overflow-x-auto rounded-lg border border-outline-variant/10">
                <table className="w-full min-w-[34rem] text-left text-[11px]">
                  <thead className="bg-surface-container-low text-[9px] uppercase tracking-wide text-on-surface-variant">
                    <tr>
                      <th scope="col" className="px-3 py-2">Observation</th>
                      <th scope="col" className="px-3 py-2 text-right">Baseline</th>
                      <th scope="col" className="px-3 py-2 text-right">Candidate</th>
                      <th scope="col" className="px-3 py-2">Evidence</th>
                    </tr>
                  </thead>
                  <tbody>
                    {comparison.observations.map(observation => (
                      <tr
                        key={observation.key}
                        className={`border-t border-outline-variant/10 ${observation.candidateObserved ? 'bg-warning/10' : ''}`}
                      >
                        <th scope="row" className="px-3 py-2 font-medium text-on-surface">{observation.label}</th>
                        <td className="px-3 py-2 text-right tabular-nums text-on-surface">{observation.baseline}</td>
                        <td className={`px-3 py-2 text-right tabular-nums ${observation.candidateObserved ? 'font-semibold text-warning' : 'text-on-surface'}`}>
                          {observation.candidate}
                        </td>
                        <td className="px-3 py-2 text-[10px] text-on-surface-variant">
                          {observationEvidence(observation)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </section>

            <section aria-labelledby="compatibility-heading" className="mt-4 rounded-lg border border-outline-variant/10 bg-surface-container-low p-3">
              <h3 id="compatibility-heading" className="text-xs font-semibold text-on-surface">Compatibility</h3>
              <p className="mt-1 text-[11px] text-on-surface-variant">
                {comparison.cohorts.length > 0
                  ? `${comparison.cohorts.length} exact ${comparison.cohorts.length === 1 ? 'cohort' : 'cohorts'} matched. ${comparison.baselineEligibleCount - comparison.baselineUnmatchedCount} baseline and ${comparison.candidateEligibleCount - comparison.candidateUnmatchedCount} candidate runs contribute to matched cohorts.`
                  : 'No exact-compatible latency cohort was found. Version-wide observations remain available above.'}
              </p>
              {(comparison.baselineUnmatchedCount > 0 || comparison.candidateUnmatchedCount > 0) && (
                <p className="mt-2 text-[11px] text-on-surface">
                  Unmatched eligible runs: {comparison.baselineUnmatchedCount} baseline, {comparison.candidateUnmatchedCount} candidate.
                  {comparison.mismatchedDimensions.length > 0
                    ? ` Different cohort facts: ${comparison.mismatchedDimensions.join(', ')}.`
                    : ''}
                </p>
              )}
              {comparison.exclusions.length > 0 && (
                <ul aria-label="Excluded run counts" className="mt-2 space-y-1 text-[11px] text-on-surface-variant">
                  {comparison.exclusions.map(exclusion => (
                    <li key={exclusion.reason}>
                      {exclusion.label}: {exclusion.baseline} baseline, {exclusion.candidate} candidate
                    </li>
                  ))}
                </ul>
              )}
            </section>

            {comparison.cohorts.map((cohort, index) => (
              <section key={cohort.key} aria-labelledby={`production-cohort-${index}`} className="mt-4">
                <h3 id={`production-cohort-${index}`} className="text-xs font-semibold text-on-surface">
                  Compatible cohort {index + 1}
                </h3>
                <p className="mt-0.5 text-[10px] leading-4 text-on-surface-variant">{cohort.label}</p>
                <p className="mt-1 text-[10px] text-on-surface-variant">
                  Runs: {cohort.baselineRunCount} baseline, {cohort.candidateRunCount} candidate. Successful latency runs: {cohort.baselineSuccessCount} baseline, {cohort.candidateSuccessCount} candidate.
                </p>
                <div className="mt-2 overflow-x-auto rounded-lg border border-outline-variant/10">
                  <table className="w-full min-w-[58rem] text-left text-[11px]">
                    <thead className="bg-surface-container-low text-[9px] uppercase tracking-wide text-on-surface-variant">
                      <tr>
                        <th scope="col" className="px-3 py-2">Metric</th>
                        <th scope="col" className="px-3 py-2 text-right">Baseline median</th>
                        <th scope="col" className="px-3 py-2 text-right">Candidate median</th>
                        <th scope="col" className="px-3 py-2 text-right">Median change</th>
                        <th scope="col" className="px-3 py-2 text-right">p95</th>
                      </tr>
                    </thead>
                    <tbody>
                      {cohort.metrics.map(metric => <MetricRow key={metric.definition.key} metric={metric} />)}
                    </tbody>
                  </table>
                </div>
              </section>
            ))}
          </>
        )}
      </div>
    </details>
  );
}
