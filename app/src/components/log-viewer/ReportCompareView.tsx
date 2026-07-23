import { useEffect, useMemo, useRef, useState, type ChangeEvent } from 'react';
import { loadBenchmarkReports } from '../../lib/benchmark';
import {
  MAX_DIAGNOSTIC_REPORT_BYTES,
  normalizeLocalBenchmarkReport,
  parseDiagnosticReportJson,
  type NormalizedBenchmarkReport,
  type NormalizedDiagnosticReport,
  type NormalizedEvaluationReport,
} from '../../lib/diagnosticReports';
import {
  compareDiagnosticReports,
  type ComparisonMetric,
  type DiagnosticComparison,
} from '../../lib/diagnosticComparison';

const MAX_SESSION_IMPORTS = 20;
const MAX_VISIBLE_METRICS = 500;
const READ_ERROR = 'The selected diagnostic report could not be read.';
const SESSION_LIMIT_ERROR = `This session can hold up to ${MAX_SESSION_IMPORTS} imported diagnostic reports.`;

interface ReportEntry {
  id: string;
  report: NormalizedDiagnosticReport;
}

function localReportEntries(): ReportEntry[] {
  return loadBenchmarkReports().flatMap((report, index) => {
    const result = normalizeLocalBenchmarkReport(report);
    return result.ok
      ? [{ id: `local-${index}-${report.createdAt}`, report: result.report }]
      : [];
  });
}

function schemaLabel(report: NormalizedDiagnosticReport): string {
  if (report.kind === 'benchmark') {
    return report.schemaVersion === 'legacy' ? 'Legacy' : `v${report.schemaVersion}`;
  }
  return `v${report.schemaVersion} · fixtures v${report.fixtureVersion}`;
}

function reportLabel(report: NormalizedDiagnosticReport): string {
  const source = report.source === 'local' ? 'Local' : 'Imported';
  const kind = report.kind === 'benchmark' ? 'benchmark' : 'evaluation';
  return `${source} ${kind} · ${new Date(report.createdAt).toLocaleString()}`;
}

function sourceClasses(source: NormalizedDiagnosticReport['source']): string {
  return source === 'local'
    ? 'bg-blue-100 text-blue-700 dark:bg-blue-900/35 dark:text-blue-300'
    : 'bg-violet-100 text-violet-700 dark:bg-violet-900/35 dark:text-violet-300';
}

function statusClasses(status: DiagnosticComparison['status']): string {
  if (status === 'compatible') {
    return 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/35 dark:text-emerald-300';
  }
  if (status === 'warning') {
    return 'bg-amber-100 text-amber-800 dark:bg-amber-900/35 dark:text-amber-200';
  }
  return 'bg-red-100 text-red-700 dark:bg-red-900/35 dark:text-red-300';
}

function titleCase(value: string): string {
  return value.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/^./, letter => letter.toUpperCase());
}

function benchmarkMachine(report: NormalizedBenchmarkReport): string {
  if (!report.environment) return report.platform;
  const machine = report.environment.chip
    ?? report.environment.hardwareModel
    ?? report.environment.architecture;
  return `${report.environment.os}${report.environment.osVersion ? ` ${report.environment.osVersion}` : ''} · ${machine}`;
}

function evaluationMachine(report: NormalizedEvaluationReport): string {
  return `${report.environment.os} · ${report.environment.machineLabel} · ${report.environment.architecture}`;
}

function benchmarkModels(report: NormalizedBenchmarkReport): string {
  if (report.models.length === 0) return 'Unavailable';
  return report.models
    .map(model => `${model.label} · ${model.backend} · ${model.accelerator}`)
    .join('; ');
}

function evaluationModels(report: NormalizedEvaluationReport): string {
  const models = Array.from(new Set(report.cases.flatMap(entry =>
    entry.model
      ? [`${entry.model.name} · ${entry.model.backend} · ${entry.model.accelerator}`]
      : [])));
  return models.length > 0 ? models.join('; ') : 'Fixture-only deterministic run';
}

function evaluationExecution(report: NormalizedEvaluationReport): string {
  const signatures = Array.from(new Set(report.cases.map(entry =>
    `${entry.fixtureOnly ? 'fixture-only' : 'runtime'} · ${entry.delivery.finalOnly ? 'final-only' : 'incremental'} · ${entry.runtime.incrementalCompletion}`)));
  return signatures.length > 0 ? signatures.join('; ') : 'Unavailable';
}

function evaluationStages(report: NormalizedEvaluationReport): string {
  const stageSets = Array.from(new Set(report.cases.map(entry =>
    entry.transformation.stages.map(stage => stage.name).join(' → '))));
  return stageSets.filter(Boolean).join('; ') || 'Unavailable';
}

function formatNumber(value: number, unit: ComparisonMetric['unit']): string {
  if (unit === 'count') return value.toLocaleString();
  const maximumFractionDigits = unit === 'ratio' ? 4 : 2;
  const formatted = value.toLocaleString(undefined, { maximumFractionDigits });
  if (unit === 'ms') return `${formatted} ms`;
  if (unit === 'microseconds') return `${formatted} µs`;
  if (unit === 'mb') return `${formatted} MB`;
  return formatted;
}

function formatDelta(value: number, unit: ComparisonMetric['unit']): string {
  const sign = value > 0 ? '+' : '';
  return `${sign}${formatNumber(value, unit)}`;
}

function MetaRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid min-w-0 grid-cols-[7rem_minmax(0,1fr)] gap-2 border-t border-outline-variant/10 py-1.5 first:border-t-0">
      <dt className="text-[10px] font-medium uppercase tracking-wide text-on-surface-variant">
        {label}
      </dt>
      <dd className="min-w-0 break-words text-xs text-on-surface">{value}</dd>
    </div>
  );
}

function ReportCard({
  heading,
  entry,
}: {
  heading: string;
  entry: ReportEntry | undefined;
}) {
  if (!entry) {
    return (
      <section className="rounded-xl border border-dashed border-outline-variant/30 bg-surface-container-lowest p-3">
        <h3 className="text-xs font-semibold text-on-surface">{heading}</h3>
        <p className="mt-3 text-xs text-on-surface-variant">Choose a report to inspect.</p>
      </section>
    );
  }

  const { report } = entry;
  const commonRows = [
    ['Date', new Date(report.createdAt).toLocaleString()],
    ['Type / schema', `${titleCase(report.kind)} · ${schemaLabel(report)}`],
  ] as const;
  const rows = report.kind === 'benchmark'
    ? [
      ...commonRows,
      ['Machine', benchmarkMachine(report)],
      ['App', `${report.appVersion} · ${report.platform}`],
      ['Preset', `${titleCase(report.preset)} · ${report.iterations} iterations`],
      ['Corpus', report.corpus
        ? `${report.corpus.language} · ${report.corpus.fixtureCount} fixtures · ${report.corpus.referenceWords} reference words`
        : 'Unavailable in legacy schema'],
      ['Execution', report.configuration
        ? report.configuration.executionPath
        : 'Unavailable in legacy schema'],
      ['Configuration', report.configuration
        ? `VAD ${report.configuration.vadThreshold} · ${report.configuration.transcriptTransformProfile} · ${report.configuration.percentileMethod} · model order ${report.configuration.modelRunOrder.join(' → ')} · shared init ${report.configuration.sharedInitOrder.join(' → ')}`
        : 'Unavailable in legacy schema'],
      ['Models', benchmarkModels(report)],
    ]
    : [
      ...commonRows,
      ['Machine', evaluationMachine(report)],
      ['App', report.environment.appVersion],
      ['Tier / corpus', `${titleCase(report.tier)} · ${report.summary.total} fixture cases`],
      ['Execution', evaluationExecution(report)],
      ['Configuration', `Stages ${evaluationStages(report)}`],
      ['Models', evaluationModels(report)],
    ];

  return (
    <section className="min-w-0 rounded-xl border border-outline-variant/10 bg-surface-container-lowest p-3 shadow-sm">
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-xs font-semibold text-on-surface">{heading}</h3>
        <span className={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${sourceClasses(report.source)}`}>
          {report.source}
        </span>
      </div>
      <dl>
        {rows.map(([label, value]) => <MetaRow key={label} label={label} value={value} />)}
      </dl>
      {report.privacyWarnings.map(warning => (
        <p
          key={warning}
          className="mt-2 rounded-lg border border-amber-500/20 bg-amber-500/10 px-2.5 py-2 text-[11px] leading-4 text-amber-800 dark:text-amber-200"
        >
          {warning}
        </p>
      ))}
    </section>
  );
}

function CompatibilityGate({
  comparison,
  candidate,
}: {
  comparison: DiagnosticComparison;
  candidate: NormalizedDiagnosticReport;
}) {
  const visibleMetrics = comparison.metrics.slice(0, MAX_VISIBLE_METRICS);
  return (
    <section aria-labelledby="report-compatibility-heading" className="rounded-xl border border-outline-variant/10 bg-surface-container-lowest p-3 shadow-sm">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3 id="report-compatibility-heading" className="text-sm font-semibold text-on-surface">
            Compatibility gate
          </h3>
          <p className="text-[11px] text-on-surface-variant">
            Compatibility is checked before deltas or recommendations are shown.
          </p>
        </div>
        <span className={`rounded-full px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide ${statusClasses(comparison.status)}`}>
          {comparison.status}
        </span>
      </div>

      {comparison.issues.length > 0 ? (
        <ul className="mt-3 space-y-2" aria-label="Compatibility findings">
          {comparison.issues.map(issue => (
            <li
              key={`${issue.code}-${issue.field}`}
              className={`rounded-lg border px-2.5 py-2 text-xs ${
                issue.severity === 'blocker'
                  ? 'border-error/20 bg-error/10 text-error'
                  : 'border-amber-500/20 bg-amber-500/10 text-amber-800 dark:text-amber-200'
              }`}
            >
              <span className="font-semibold">{titleCase(issue.severity)} · {issue.field}</span>
              <span className="ml-1">{issue.message}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-3 rounded-lg border border-emerald-500/20 bg-emerald-500/10 px-2.5 py-2 text-xs text-emerald-700 dark:text-emerald-300">
          These reports are compatible for like-for-like deltas.
        </p>
      )}

      {!comparison.deltasAllowed ? (
        <p className="mt-3 text-xs font-medium text-error">
          Deltas and recommendations are unavailable while compatibility blockers remain.
        </p>
      ) : (
        <>
          <div data-testid="report-metrics-scroller" className="mt-3 overflow-x-auto">
            <table className="w-full min-w-[42rem] border-separate border-spacing-0 text-left text-xs">
              <thead>
                <tr className="text-[10px] uppercase tracking-wide text-on-surface-variant">
                  <th className="border-b border-outline-variant/20 px-2 py-2 font-medium">Metric</th>
                  <th className="border-b border-outline-variant/20 px-2 py-2 text-right font-medium">Baseline</th>
                  <th className="border-b border-outline-variant/20 px-2 py-2 text-right font-medium">Candidate</th>
                  <th className="border-b border-outline-variant/20 px-2 py-2 text-right font-medium">Absolute Δ</th>
                  <th className="border-b border-outline-variant/20 px-2 py-2 text-right font-medium">Percentage Δ</th>
                </tr>
              </thead>
              <tbody>
                {visibleMetrics.map(metric => (
                  <tr key={metric.key} className="align-top">
                    <th className="border-b border-outline-variant/10 px-2 py-2 font-medium text-on-surface">
                      {metric.label}
                      <span className="block max-w-72 break-all font-mono text-[9px] font-normal text-on-surface-variant">
                        {metric.scope}
                      </span>
                    </th>
                    <td className="border-b border-outline-variant/10 px-2 py-2 text-right tabular-nums text-on-surface">
                      {formatNumber(metric.baseline, metric.unit)}
                    </td>
                    <td className="border-b border-outline-variant/10 px-2 py-2 text-right tabular-nums text-on-surface">
                      {formatNumber(metric.candidate, metric.unit)}
                    </td>
                    <td className="border-b border-outline-variant/10 px-2 py-2 text-right tabular-nums text-on-surface">
                      {formatDelta(metric.absoluteDelta, metric.unit)}
                    </td>
                    <td className="border-b border-outline-variant/10 px-2 py-2 text-right tabular-nums text-on-surface">
                      {metric.percentageDelta === null
                        ? <span className="text-on-surface-variant">Unavailable · zero baseline</span>
                        : formatDelta(metric.percentageDelta, 'ratio') + '%'}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {comparison.metrics.length > MAX_VISIBLE_METRICS && (
            <p className="mt-2 text-[11px] text-on-surface-variant">
              Showing the first {MAX_VISIBLE_METRICS.toLocaleString()} of {comparison.metrics.length.toLocaleString()} bounded metrics.
            </p>
          )}
          {comparison.metrics.length === 0 && (
            <p className="mt-3 text-xs text-on-surface-variant">
              Compatible reports contain no shared measured metrics. Missing values remain unavailable rather than becoming zero.
            </p>
          )}
        </>
      )}

      <div className="mt-3 border-t border-outline-variant/10 pt-3">
        <h4 className="text-xs font-semibold text-on-surface">Recommendation eligibility</h4>
        <p className="mt-1 text-xs text-on-surface-variant">
          {comparison.recommendationAllowed
            ? 'Eligible. Benchmark recommendations may be inspected without compatibility blockers or warnings; evaluation reports remain delta-only.'
            : 'Not eligible. Murmur will not rank or recommend from this comparison.'}
        </p>
        {comparison.recommendationAllowed && candidate.kind === 'benchmark' && (
          <dl className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-3">
            {([
              ['Fastest', candidate.recommendations.fastest],
              ['Most accurate', candidate.recommendations.mostAccurate],
              ['Balanced', candidate.recommendations.balanced],
            ] as const).map(([label, value]) => (
              <div key={label} className="rounded-lg bg-surface-container px-2.5 py-2">
                <dt className="text-[10px] font-medium uppercase tracking-wide text-on-surface-variant">{label}</dt>
                <dd className="mt-0.5 break-words text-xs font-medium text-on-surface">{value ?? 'Unavailable'}</dd>
              </div>
            ))}
          </dl>
        )}
      </div>
    </section>
  );
}

export function ReportCompareView() {
  const [reports, setReports] = useState<ReportEntry[]>(localReportEntries);
  const [baselineId, setBaselineId] = useState('');
  const [candidateId, setCandidateId] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const importSequence = useRef(0);

  useEffect(() => {
    const ids = new Set(reports.map(entry => entry.id));
    if (!ids.has(baselineId)) setBaselineId(reports[0]?.id ?? '');
    if (!ids.has(candidateId) || candidateId === baselineId) {
      setCandidateId(reports.find(entry => entry.id !== baselineId)?.id ?? '');
    }
  }, [baselineId, candidateId, reports]);

  const baseline = reports.find(entry => entry.id === baselineId);
  const candidate = reports.find(entry => entry.id === candidateId);
  const comparison = useMemo(() => (
    baseline && candidate && baseline.id !== candidate.id
      ? compareDiagnosticReports(baseline.report, candidate.report)
      : null
  ), [baseline, candidate]);

  const importReport = async (event: ChangeEvent<HTMLInputElement>) => {
    const input = event.currentTarget;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;

    setError(null);
    setNotice(null);
    if (file.size > MAX_DIAGNOSTIC_REPORT_BYTES) {
      setError('Diagnostic reports are limited to 8 MiB.');
      return;
    }
    if (reports.filter(entry => entry.report.source === 'imported').length >= MAX_SESSION_IMPORTS) {
      setError(SESSION_LIMIT_ERROR);
      return;
    }

    let contents: string;
    try {
      contents = await file.text();
    } catch {
      setError(READ_ERROR);
      return;
    }
    const result = parseDiagnosticReportJson(contents, file.size);
    if (!result.ok) {
      setError(result.error.message);
      return;
    }

    importSequence.current += 1;
    const next: ReportEntry = {
      id: `imported-${importSequence.current}`,
      report: result.report,
    };
    setReports(current => [...current, next]);
    if (!baselineId) setBaselineId(next.id);
    else setCandidateId(next.id);
    setNotice(`Imported ${result.report.kind} ${schemaLabel(result.report)} into this session.`);
  };

  const clearImports = () => {
    setReports(current => current.filter(entry => entry.report.source === 'local'));
    setError(null);
    setNotice('Session imports cleared. Source files and saved Performance Lab history were not changed.');
  };

  const importedCount = reports.filter(entry => entry.report.source === 'imported').length;

  return (
    <div className="flex min-h-full flex-col gap-4 p-4">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold text-on-surface">Report comparison</h2>
          <p className="text-[11px] text-on-surface-variant">
            Compare local Performance Lab history or explicit JSON imports · imports stay in this Diagnostics session
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <label className="cursor-pointer rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary shadow-sm hover:opacity-90 focus-within:ring-2 focus-within:ring-primary focus-within:ring-offset-2">
            Import JSON
            <input
              data-testid="diagnostic-report-input"
              type="file"
              accept=".json,application/json"
              className="sr-only"
              onChange={event => void importReport(event)}
            />
          </label>
          <button
            type="button"
            disabled={importedCount === 0}
            onClick={clearImports}
            className="rounded-lg border border-outline-variant/20 px-3 py-1.5 text-xs font-medium text-on-surface-variant hover:bg-surface-container focus:outline-none focus-visible:ring-2 focus-visible:ring-primary disabled:cursor-not-allowed disabled:opacity-50"
          >
            Clear imports
          </button>
        </div>
      </div>

      {error && (
        <div role="alert" className="rounded-xl border border-error/20 bg-error/10 px-3 py-2 text-xs text-error">
          {error}
        </div>
      )}
      {notice && (
        <div aria-live="polite" className="rounded-xl border border-primary/15 bg-primary/10 px-3 py-2 text-xs text-on-surface">
          {notice}
        </div>
      )}

      {reports.length === 0 ? (
        <div className="flex min-h-48 flex-col items-center justify-center rounded-xl border border-dashed border-outline-variant/30 bg-surface-container-lowest px-6 text-center">
          <p className="text-sm font-medium text-on-surface">No diagnostic reports available</p>
          <p className="mt-1 max-w-md text-xs text-on-surface-variant">
            Import a Murmur benchmark or evaluation JSON report. The file is checked locally and is not added to saved history.
          </p>
        </div>
      ) : (
        <>
          <div data-testid="report-selection-grid" className="grid grid-cols-1 gap-3 lg:grid-cols-2">
            <label className="text-[10px] font-medium uppercase tracking-wider text-on-surface-variant">
              Baseline
              <select
                aria-label="Baseline report"
                value={baselineId}
                onChange={event => setBaselineId(event.target.value)}
                className="mt-1 block w-full rounded-lg border border-outline-variant/20 bg-surface-container-lowest px-2.5 py-2 text-xs normal-case tracking-normal text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
              >
                {reports.map(entry => <option key={entry.id} value={entry.id}>{reportLabel(entry.report)}</option>)}
              </select>
            </label>
            <label className="text-[10px] font-medium uppercase tracking-wider text-on-surface-variant">
              Candidate
              <select
                aria-label="Candidate report"
                value={candidateId}
                onChange={event => setCandidateId(event.target.value)}
                className="mt-1 block w-full rounded-lg border border-outline-variant/20 bg-surface-container-lowest px-2.5 py-2 text-xs normal-case tracking-normal text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
              >
                <option value="">Choose a second report</option>
                {reports.map(entry => <option key={entry.id} value={entry.id}>{reportLabel(entry.report)}</option>)}
              </select>
            </label>
          </div>

          <div data-testid="report-summary-grid" className="grid grid-cols-1 gap-3 lg:grid-cols-2">
            <ReportCard heading="Baseline report" entry={baseline} />
            <ReportCard heading="Candidate report" entry={candidate} />
          </div>

          {baseline && candidate && baseline.id === candidate.id && (
            <div role="status" className="rounded-xl border border-amber-500/20 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
              Choose two different reports to compare.
            </div>
          )}
          {comparison && candidate && (
            <CompatibilityGate comparison={comparison} candidate={candidate.report} />
          )}
        </>
      )}
    </div>
  );
}
