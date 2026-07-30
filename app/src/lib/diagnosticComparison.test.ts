import { describe, expect, it } from 'vitest';
import benchmarkLegacy from './__fixtures__/diagnostic-reports/benchmark-legacy.json';
import benchmarkV2 from './__fixtures__/diagnostic-reports/benchmark-v2.json';
import evaluationDeterministic from './__fixtures__/diagnostic-reports/evaluation-v1-deterministic.json';
import evaluationHardware from './__fixtures__/diagnostic-reports/evaluation-v1-hardware.json';
import { compareDiagnosticReports } from './diagnosticComparison';
import { parseDiagnosticReportJson } from './diagnosticReports';
import type { NormalizedDiagnosticReport } from './diagnosticReports';

function imported(value: unknown): NormalizedDiagnosticReport {
  const contents = JSON.stringify(value);
  const result = parseDiagnosticReportJson(
    contents,
    new TextEncoder().encode(contents).byteLength,
  );
  if (!result.ok) throw new Error(result.error.code);
  return result.report;
}

describe('compareDiagnosticReports', () => {
  it('computes like-for-like benchmark absolute and percentage deltas', () => {
    const baseline = imported(benchmarkV2);
    const candidate = structuredClone(baseline);
    if (candidate.kind !== 'benchmark') throw new Error('benchmark fixture expected');
    candidate.sharedInitMs = 900;
    candidate.models[0].warmMedianMs = 99;
    candidate.models[0].fixtures[0].wordErrorRate = 0.08;

    const comparison = compareDiagnosticReports(baseline, candidate);
    expect(comparison.status).toBe('compatible');
    expect(comparison.deltasAllowed).toBe(true);
    expect(comparison.recommendationAllowed).toBe(true);
    expect(comparison.metrics.find((entry) => entry.key === 'benchmark.sharedInitMs'))
      .toMatchObject({
        baseline: 1200,
        candidate: 900,
        absoluteDelta: -300,
        percentageDelta: -25,
      });
    expect(comparison.metrics.find((entry) => entry.label === 'Warm median'))
      .toMatchObject({ baseline: 110, candidate: 99, absoluteDelta: -11 });
  });

  it('returns no percentage delta when the baseline is zero', () => {
    const baseline = imported(evaluationDeterministic);
    const candidate = structuredClone(baseline);
    if (candidate.kind !== 'evaluation') throw new Error('evaluation fixture expected');
    candidate.summary.aggregateRawWer = 0.1;

    const comparison = compareDiagnosticReports(baseline, candidate);
    expect(comparison.metrics.find((entry) => entry.key === 'evaluation.summary.aggregateRawWer'))
      .toMatchObject({
        baseline: 0,
        candidate: 0.1,
        absoluteDelta: 0.1,
        percentageDelta: null,
      });
  });

  it('blocks benchmark deltas when corpus, configuration, or model identity differs', () => {
    const baseline = imported(benchmarkV2);
    const candidate = structuredClone(baseline);
    if (candidate.kind !== 'benchmark') throw new Error('benchmark fixture expected');
    if (!candidate.corpus || !candidate.configuration) throw new Error('v2 fixture expected');
    candidate.corpus.fixtureIds = ['different'];
    candidate.configuration.vadThreshold = 0.6;
    candidate.models[0].backend = 'different-backend';

    const comparison = compareDiagnosticReports(baseline, candidate);
    expect(comparison.status).toBe('blocked');
    expect(comparison.deltasAllowed).toBe(false);
    expect(comparison.recommendationAllowed).toBe(false);
    expect(comparison.metrics).toEqual([]);
    expect(comparison.issues.map((entry) => entry.code)).toEqual(expect.arrayContaining([
      'corpus_mismatch',
      'configuration_mismatch',
      'model_set_mismatch',
    ]));
  });

  it('blocks incomplete results and per-model fixture mismatches', () => {
    const baseline = imported(benchmarkV2);
    const failed = structuredClone(baseline);
    if (failed.kind !== 'benchmark') throw new Error('benchmark fixture expected');
    failed.models[0].succeeded = false;
    expect(compareDiagnosticReports(baseline, failed).issues).toContainEqual(
      expect.objectContaining({ code: 'benchmark_result_incomplete', severity: 'blocker' }),
    );

    const missingFixture = structuredClone(baseline);
    if (missingFixture.kind !== 'benchmark') throw new Error('benchmark fixture expected');
    missingFixture.models[0].fixtures = [];
    expect(compareDiagnosticReports(baseline, missingFixture).issues).toContainEqual(
      expect.objectContaining({ code: 'corpus_mismatch', field: 'models.fixtures' }),
    );
  });

  it('imports legacy benchmarks but blocks unproven comparisons', () => {
    const baseline = imported(benchmarkLegacy);
    const candidate = structuredClone(baseline);
    const comparison = compareDiagnosticReports(baseline, candidate);

    expect(comparison).toMatchObject({
      status: 'blocked',
      deltasAllowed: false,
      recommendationAllowed: false,
      metrics: [],
    });
    expect(comparison.issues).toContainEqual(expect.objectContaining({
      code: 'legacy_context_missing',
      severity: 'blocker',
    }));
  });

  it('keeps machine and app-version differences visible and disables recommendations', () => {
    const baseline = imported(benchmarkV2);
    const candidate = structuredClone(baseline);
    if (candidate.kind !== 'benchmark' || !candidate.environment) {
      throw new Error('v2 benchmark fixture expected');
    }
    candidate.appVersion = '0.20.0';
    candidate.environment.chip = 'Apple M3';

    const comparison = compareDiagnosticReports(baseline, candidate);
    expect(comparison.status).toBe('warning');
    expect(comparison.deltasAllowed).toBe(true);
    expect(comparison.recommendationAllowed).toBe(false);
    expect(comparison.metrics.length).toBeGreaterThan(0);
    expect(comparison.issues.map((entry) => entry.code)).toEqual([
      'app_version_mismatch',
      'machine_mismatch',
    ]);
  });

  it('blocks benchmark-to-evaluation comparisons without producing metrics', () => {
    const comparison = compareDiagnosticReports(
      imported(benchmarkV2),
      imported(evaluationDeterministic),
    );
    expect(comparison).toEqual({
      status: 'blocked',
      issues: [{
        code: 'report_type_mismatch',
        severity: 'blocker',
        field: 'kind',
        message: 'Benchmark and evaluation reports use different metric semantics.',
      }],
      deltasAllowed: false,
      recommendationAllowed: false,
      metrics: [],
    });
  });

  it('compares evaluation summaries, cases, latencies, and stage durations', () => {
    const baseline = imported(evaluationDeterministic);
    const candidate = structuredClone(baseline);
    if (candidate.kind !== 'evaluation') throw new Error('evaluation fixture expected');
    candidate.cases[0].latency.totalMs = 15;
    candidate.cases[0].transformation.stages[0].durationUs = 90;

    const comparison = compareDiagnosticReports(baseline, candidate);
    expect(comparison.status).toBe('compatible');
    expect(comparison.metrics.find((entry) => entry.key.endsWith('.totalMs')))
      .toMatchObject({ baseline: 18, candidate: 15, absoluteDelta: -3 });
    expect(comparison.metrics.find((entry) => entry.unit === 'microseconds'))
      .toMatchObject({ baseline: 120, candidate: 90, absoluteDelta: -30 });
  });

  it('blocks evaluation tier, fixture, model, and execution-semantic mismatches', () => {
    const deterministic = imported(evaluationDeterministic);
    const hardware = imported(evaluationHardware);
    const tierComparison = compareDiagnosticReports(deterministic, hardware);
    expect(tierComparison.status).toBe('blocked');
    expect(tierComparison.issues.map((entry) => entry.code)).toEqual(expect.arrayContaining([
      'evaluation_tier_mismatch',
      'fixture_set_mismatch',
    ]));

    const baseline = imported(evaluationHardware);
    const candidate = structuredClone(baseline);
    if (candidate.kind !== 'evaluation' || !candidate.cases[0].model) {
      throw new Error('hardware evaluation fixture expected');
    }
    candidate.cases[0].model.accelerator = 'CPU';
    candidate.cases[0].delivery.finalOnly = false;
    const semanticComparison = compareDiagnosticReports(baseline, candidate);
    expect(semanticComparison.status).toBe('blocked');
    expect(semanticComparison.metrics).toEqual([]);
    expect(semanticComparison.issues.map((entry) => entry.code)).toEqual(expect.arrayContaining([
      'evaluation_model_mismatch',
      'evaluation_execution_mismatch',
    ]));
  });

  it('blocks recommendations for failed or incomplete evaluation results', () => {
    const baseline = imported(evaluationDeterministic);
    const candidate = structuredClone(baseline);
    if (candidate.kind !== 'evaluation') throw new Error('evaluation fixture expected');
    candidate.cases[0].status = 'failed';
    candidate.cases[0].complete = false;

    const comparison = compareDiagnosticReports(baseline, candidate);
    expect(comparison).toMatchObject({
      status: 'blocked',
      deltasAllowed: false,
      recommendationAllowed: false,
      metrics: [],
    });
    expect(comparison.issues).toContainEqual(expect.objectContaining({
      code: 'evaluation_result_incomplete',
      severity: 'blocker',
    }));
  });
});
