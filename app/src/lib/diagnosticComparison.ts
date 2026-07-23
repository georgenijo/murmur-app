import type {
  NormalizedBenchmarkModel,
  NormalizedBenchmarkReport,
  NormalizedDiagnosticReport,
  NormalizedEvaluationCase,
  NormalizedEvaluationReport,
} from './diagnosticReports';

export type CompatibilitySeverity = 'warning' | 'blocker';
export type CompatibilityStatus = 'compatible' | 'warning' | 'blocked';

export type CompatibilityIssueCode =
  | 'report_type_mismatch'
  | 'schema_mismatch'
  | 'legacy_context_missing'
  | 'preset_mismatch'
  | 'iteration_mismatch'
  | 'corpus_mismatch'
  | 'configuration_mismatch'
  | 'model_set_mismatch'
  | 'benchmark_result_incomplete'
  | 'evaluation_tier_mismatch'
  | 'fixture_set_mismatch'
  | 'evaluation_model_mismatch'
  | 'evaluation_execution_mismatch'
  | 'machine_mismatch'
  | 'app_version_mismatch';

export interface CompatibilityIssue {
  code: CompatibilityIssueCode;
  severity: CompatibilitySeverity;
  field: string;
  message: string;
}

export type ComparisonMetricUnit = 'ms' | 'microseconds' | 'ratio' | 'count' | 'mb';
export type MetricDirection = 'lower_is_better' | 'higher_is_better' | 'neutral';

export interface ComparisonMetric {
  key: string;
  scope: string;
  label: string;
  unit: ComparisonMetricUnit;
  direction: MetricDirection;
  baseline: number;
  candidate: number;
  absoluteDelta: number;
  percentageDelta: number | null;
}

export interface DiagnosticComparison {
  status: CompatibilityStatus;
  issues: CompatibilityIssue[];
  deltasAllowed: boolean;
  recommendationAllowed: boolean;
  metrics: ComparisonMetric[];
}

function sameStrings(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((entry, index) => entry === right[index]);
}

function sameStringSet(left: readonly string[], right: readonly string[]): boolean {
  return sameStrings([...left].sort(), [...right].sort());
}

function benchmarkModelIdentity(model: NormalizedBenchmarkModel): string {
  return `${model.modelName}\u0000${model.backend}\u0000${model.accelerator}`;
}

function evaluationModelIdentity(entry: NormalizedEvaluationCase): string {
  if (!entry.model) return 'none';
  return `${entry.model.name}\u0000${entry.model.backend}\u0000${entry.model.accelerator}`;
}

function addIssue(
  issues: CompatibilityIssue[],
  issue: CompatibilityIssue,
): void {
  if (!issues.some((entry) => entry.code === issue.code && entry.field === issue.field)) {
    issues.push(issue);
  }
}

function blocker(
  issues: CompatibilityIssue[],
  code: CompatibilityIssueCode,
  field: string,
  message: string,
): void {
  addIssue(issues, { code, severity: 'blocker', field, message });
}

function warning(
  issues: CompatibilityIssue[],
  code: CompatibilityIssueCode,
  field: string,
  message: string,
): void {
  addIssue(issues, { code, severity: 'warning', field, message });
}

function compareBenchmarkCompatibility(
  baseline: NormalizedBenchmarkReport,
  candidate: NormalizedBenchmarkReport,
): CompatibilityIssue[] {
  const issues: CompatibilityIssue[] = [];
  if (baseline.schemaVersion !== candidate.schemaVersion) {
    blocker(
      issues,
      'schema_mismatch',
      'schemaVersion',
      'Benchmark schema versions differ, so their semantics are not guaranteed to match.',
    );
  }
  if (baseline.schemaVersion === 'legacy' || candidate.schemaVersion === 'legacy') {
    blocker(
      issues,
      'legacy_context_missing',
      'context',
      'Legacy benchmark metadata is incomplete, so a like-for-like comparison cannot be proven.',
    );
  }
  if (baseline.preset !== candidate.preset) {
    blocker(issues, 'preset_mismatch', 'preset', 'Benchmark presets differ.');
  }
  if (baseline.iterations !== candidate.iterations) {
    blocker(issues, 'iteration_mismatch', 'iterations', 'Measured iteration counts differ.');
  }
  if (baseline.corpus && candidate.corpus) {
    if (baseline.corpus.language !== candidate.corpus.language
      || baseline.corpus.fixtureCount !== candidate.corpus.fixtureCount
      || baseline.corpus.referenceWords !== candidate.corpus.referenceWords
      || !sameStrings(baseline.corpus.fixtureIds, candidate.corpus.fixtureIds)) {
      blocker(
        issues,
        'corpus_mismatch',
        'corpus',
        'Benchmark corpus identities or reference counts differ.',
      );
    }
  }
  if (baseline.configuration && candidate.configuration) {
    const left = baseline.configuration;
    const right = candidate.configuration;
    if (left.vadThreshold !== right.vadThreshold
      || left.executionPath !== right.executionPath
      || left.transcriptTransformProfile !== right.transcriptTransformProfile
      || left.percentileMethod !== right.percentileMethod
      || !sameStrings(left.modelRunOrder, right.modelRunOrder)
      || !sameStrings(left.sharedInitOrder, right.sharedInitOrder)) {
      blocker(
        issues,
        'configuration_mismatch',
        'configuration',
        'Benchmark execution, VAD, transform, percentile, or run-order configuration differs.',
      );
    }
  }

  const baselineModels = baseline.models.map(benchmarkModelIdentity);
  const candidateModels = candidate.models.map(benchmarkModelIdentity);
  if (!sameStringSet(baselineModels, candidateModels)) {
    blocker(
      issues,
      'model_set_mismatch',
      'models',
      'Benchmark model, backend, or accelerator identities differ.',
    );
  } else {
    const candidateByIdentity = new Map(
      candidate.models.map((entry) => [benchmarkModelIdentity(entry), entry]),
    );
    for (const left of baseline.models) {
      const right = candidateByIdentity.get(benchmarkModelIdentity(left));
      if (!right) continue;
      if (!left.succeeded || !right.succeeded) {
        blocker(
          issues,
          'benchmark_result_incomplete',
          'models.results',
          'One or more benchmark model results are incomplete or failed.',
        );
      }
      if (!sameStringSet(
        left.fixtures.map((entry) => entry.fixtureId),
        right.fixtures.map((entry) => entry.fixtureId),
      )) {
        blocker(
          issues,
          'corpus_mismatch',
          'models.fixtures',
          'Per-model benchmark fixture identities differ.',
        );
      }
    }
  }

  if (baseline.appVersion !== candidate.appVersion) {
    warning(
      issues,
      'app_version_mismatch',
      'appVersion',
      'Murmur app versions differ; implementation changes may affect the result.',
    );
  }
  if (baseline.platform !== candidate.platform
    || JSON.stringify(baseline.environment) !== JSON.stringify(candidate.environment)) {
    warning(
      issues,
      'machine_mismatch',
      'environment',
      'Machine or operating-system metadata differs.',
    );
  }
  return issues;
}

function evaluationExecutionSignature(entry: NormalizedEvaluationCase): string {
  return JSON.stringify({
    fixtureOnly: entry.fixtureOnly,
    stageNames: entry.transformation.stages.map((stage) => stage.name),
    firstPartialApplicability: entry.delivery.firstPartialApplicability,
    finalOnly: entry.delivery.finalOnly,
    incrementalCompletion: entry.runtime.incrementalCompletion,
  });
}

function compareEvaluationCompatibility(
  baseline: NormalizedEvaluationReport,
  candidate: NormalizedEvaluationReport,
): CompatibilityIssue[] {
  const issues: CompatibilityIssue[] = [];
  if (baseline.schemaVersion !== candidate.schemaVersion
    || baseline.fixtureVersion !== candidate.fixtureVersion) {
    blocker(
      issues,
      'schema_mismatch',
      'schemaVersion',
      'Evaluation report or fixture schema versions differ.',
    );
  }
  if (baseline.tier !== candidate.tier) {
    blocker(
      issues,
      'evaluation_tier_mismatch',
      'tier',
      'Deterministic and hardware evaluation tiers cannot be compared.',
    );
  }

  const baselineIds = baseline.cases.map((entry) => entry.id);
  const candidateIds = candidate.cases.map((entry) => entry.id);
  if (!sameStringSet(baselineIds, candidateIds)) {
    blocker(
      issues,
      'fixture_set_mismatch',
      'cases',
      'Evaluation fixture IDs differ.',
    );
  } else {
    const candidateById = new Map(candidate.cases.map((entry) => [entry.id, entry]));
    for (const left of baseline.cases) {
      const right = candidateById.get(left.id);
      if (!right) continue;
      if (evaluationModelIdentity(left) !== evaluationModelIdentity(right)) {
        blocker(
          issues,
          'evaluation_model_mismatch',
          'cases.model',
          'Evaluation model, backend, or accelerator identities differ.',
        );
      }
      if (evaluationExecutionSignature(left) !== evaluationExecutionSignature(right)) {
        blocker(
          issues,
          'evaluation_execution_mismatch',
          'cases.execution',
          'Evaluation execution or stage semantics differ.',
        );
      }
    }
  }

  if (baseline.environment.appVersion !== candidate.environment.appVersion) {
    warning(
      issues,
      'app_version_mismatch',
      'appVersion',
      'Murmur app versions differ; implementation changes may affect the result.',
    );
  }
  const leftMachine = {
    os: baseline.environment.os,
    architecture: baseline.environment.architecture,
    machineLabel: baseline.environment.machineLabel,
    logicalCpus: baseline.environment.logicalCpus,
  };
  const rightMachine = {
    os: candidate.environment.os,
    architecture: candidate.environment.architecture,
    machineLabel: candidate.environment.machineLabel,
    logicalCpus: candidate.environment.logicalCpus,
  };
  if (JSON.stringify(leftMachine) !== JSON.stringify(rightMachine)) {
    warning(
      issues,
      'machine_mismatch',
      'environment',
      'Machine or operating-system metadata differs.',
    );
  }
  return issues;
}

function metric(
  key: string,
  scope: string,
  label: string,
  unit: ComparisonMetricUnit,
  direction: MetricDirection,
  baseline: number | null,
  candidate: number | null,
): ComparisonMetric | null {
  if (baseline === null || candidate === null) return null;
  const absoluteDelta = candidate - baseline;
  return {
    key,
    scope,
    label,
    unit,
    direction,
    baseline,
    candidate,
    absoluteDelta,
    percentageDelta: baseline === 0 ? null : (absoluteDelta / Math.abs(baseline)) * 100,
  };
}

function pushMetric(
  metrics: ComparisonMetric[],
  value: ComparisonMetric | null,
): void {
  if (value) metrics.push(value);
}

function benchmarkMetrics(
  baseline: NormalizedBenchmarkReport,
  candidate: NormalizedBenchmarkReport,
): ComparisonMetric[] {
  const metrics: ComparisonMetric[] = [];
  pushMetric(metrics, metric(
    'benchmark.sharedInitMs',
    'report',
    'Shared initialization',
    'ms',
    'lower_is_better',
    baseline.sharedInitMs,
    candidate.sharedInitMs,
  ));
  const candidateModels = new Map(
    candidate.models.map((entry) => [benchmarkModelIdentity(entry), entry]),
  );
  for (const left of baseline.models) {
    const right = candidateModels.get(benchmarkModelIdentity(left));
    if (!right) continue;
    const scope = `model:${left.modelName}:${left.backend}:${left.accelerator}`;
    const values: Array<[
      string,
      string,
      ComparisonMetricUnit,
      MetricDirection,
      number | null,
      number | null,
    ]> = [
      ['modelLoadMs', 'Model load', 'ms', 'lower_is_better', left.modelLoadMs, right.modelLoadMs],
      ['firstInferenceMs', 'First inference', 'ms', 'lower_is_better', left.firstInferenceMs, right.firstInferenceMs],
      ['warmMedianMs', 'Warm median', 'ms', 'lower_is_better', left.warmMedianMs, right.warmMedianMs],
      ['warmP95Ms', 'Warm p95', 'ms', 'lower_is_better', left.warmP95Ms, right.warmP95Ms],
      ['realtimeFactor', 'Realtime factor', 'ratio', 'lower_is_better', left.realtimeFactor, right.realtimeFactor],
      ['wordErrorRate', 'Raw WER', 'ratio', 'lower_is_better', left.wordErrorRate, right.wordErrorRate],
      ['normalizedWordErrorRate', 'Normalized WER', 'ratio', 'lower_is_better', left.normalizedWordErrorRate, right.normalizedWordErrorRate],
      ['deliveredWordErrorRate', 'Delivered raw WER', 'ratio', 'lower_is_better', left.deliveredWordErrorRate, right.deliveredWordErrorRate],
      ['deliveredNormalizedWordErrorRate', 'Delivered normalized WER', 'ratio', 'lower_is_better', left.deliveredNormalizedWordErrorRate, right.deliveredNormalizedWordErrorRate],
      ['memoryDeltaMb', 'Observed memory delta', 'mb', 'lower_is_better', left.memoryDeltaMb, right.memoryDeltaMb],
    ];
    for (const [key, label, unit, direction, baselineValue, candidateValue] of values) {
      pushMetric(metrics, metric(
        `benchmark.${scope}.${key}`,
        scope,
        label,
        unit,
        direction,
        baselineValue,
        candidateValue,
      ));
    }

    const rightFixtures = new Map(right.fixtures.map((entry) => [entry.fixtureId, entry]));
    for (const leftFixture of left.fixtures) {
      const rightFixture = rightFixtures.get(leftFixture.fixtureId);
      if (!rightFixture) continue;
      const fixtureScope = `${scope}:fixture:${leftFixture.fixtureId}`;
      const fixtureValues: Array<[
        string,
        string,
        ComparisonMetricUnit,
        number,
        number,
      ]> = [
        ['warmMedianMs', 'Warm median', 'ms', leftFixture.warmMedianMs, rightFixture.warmMedianMs],
        ['warmP95Ms', 'Warm p95', 'ms', leftFixture.warmP95Ms, rightFixture.warmP95Ms],
        ['realtimeFactor', 'Realtime factor', 'ratio', leftFixture.realtimeFactor, rightFixture.realtimeFactor],
        ['wordErrorRate', 'Raw WER', 'ratio', leftFixture.wordErrorRate, rightFixture.wordErrorRate],
        ['normalizedWordErrorRate', 'Normalized WER', 'ratio', leftFixture.normalizedWordErrorRate, rightFixture.normalizedWordErrorRate],
        ['deliveredWordErrorRate', 'Delivered raw WER', 'ratio', leftFixture.deliveredWordErrorRate, rightFixture.deliveredWordErrorRate],
        ['deliveredNormalizedWordErrorRate', 'Delivered normalized WER', 'ratio', leftFixture.deliveredNormalizedWordErrorRate, rightFixture.deliveredNormalizedWordErrorRate],
      ];
      for (const [key, label, unit, baselineValue, candidateValue] of fixtureValues) {
        pushMetric(metrics, metric(
          `benchmark.${fixtureScope}.${key}`,
          fixtureScope,
          label,
          unit,
          'lower_is_better',
          baselineValue,
          candidateValue,
        ));
      }
    }
  }
  return metrics;
}

function evaluationMetrics(
  baseline: NormalizedEvaluationReport,
  candidate: NormalizedEvaluationReport,
): ComparisonMetric[] {
  const metrics: ComparisonMetric[] = [];
  const summaryValues: Array<[
    string,
    string,
    ComparisonMetricUnit,
    MetricDirection,
    number | null,
    number | null,
  ]> = [
    ['passed', 'Passed cases', 'count', 'higher_is_better', baseline.summary.passed, candidate.summary.passed],
    ['failed', 'Failed cases', 'count', 'lower_is_better', baseline.summary.failed, candidate.summary.failed],
    ['skipped', 'Skipped cases', 'count', 'lower_is_better', baseline.summary.skipped, candidate.summary.skipped],
    ['aggregateRawWer', 'Aggregate raw WER', 'ratio', 'lower_is_better', baseline.summary.aggregateRawWer, candidate.summary.aggregateRawWer],
    ['aggregateNormalizedWer', 'Aggregate normalized WER', 'ratio', 'lower_is_better', baseline.summary.aggregateNormalizedWer, candidate.summary.aggregateNormalizedWer],
    ['aggregateCer', 'Aggregate CER', 'ratio', 'lower_is_better', baseline.summary.aggregateCer, candidate.summary.aggregateCer],
    ['transformationMatchRate', 'Transformation match rate', 'ratio', 'higher_is_better', baseline.summary.transformationMatchRate, candidate.summary.transformationMatchRate],
    ['commandExactMatchRate', 'Command exact-match rate', 'ratio', 'higher_is_better', baseline.summary.commandExactMatchRate, candidate.summary.commandExactMatchRate],
    ['noChangePreservationRate', 'No-change preservation rate', 'ratio', 'higher_is_better', baseline.summary.noChangePreservationRate, candidate.summary.noChangePreservationRate],
    ['deliveryMatchRate', 'Delivery match rate', 'ratio', 'higher_is_better', baseline.summary.deliveryMatchRate, candidate.summary.deliveryMatchRate],
  ];
  for (const [key, label, unit, direction, baselineValue, candidateValue] of summaryValues) {
    pushMetric(metrics, metric(
      `evaluation.summary.${key}`,
      'summary',
      label,
      unit,
      direction,
      baselineValue,
      candidateValue,
    ));
  }

  const candidateCases = new Map(candidate.cases.map((entry) => [entry.id, entry]));
  for (const left of baseline.cases) {
    const right = candidateCases.get(left.id);
    if (!right) continue;
    const scope = `case:${left.id}`;
    const caseValues: Array<[
      string,
      string,
      ComparisonMetricUnit,
      MetricDirection,
      number | null,
      number | null,
    ]> = [
      ['rawWer', 'Raw WER', 'ratio', 'lower_is_better', left.recognition.rawWer, right.recognition.rawWer],
      ['normalizedWer', 'Normalized WER', 'ratio', 'lower_is_better', left.recognition.normalizedWer, right.recognition.normalizedWer],
      ['cer', 'CER', 'ratio', 'lower_is_better', left.recognition.cer, right.recognition.cer],
      ['rawAsrMs', 'Raw ASR', 'ms', 'lower_is_better', left.latency.rawAsrMs, right.latency.rawAsrMs],
      ['transformationMs', 'Transformation', 'ms', 'lower_is_better', left.latency.transformationMs, right.latency.transformationMs],
      ['finalizationMs', 'Finalization', 'ms', 'lower_is_better', left.latency.finalizationMs, right.latency.finalizationMs],
      ['deliveryMs', 'Delivery', 'ms', 'lower_is_better', left.latency.deliveryMs, right.latency.deliveryMs],
      ['totalMs', 'Total', 'ms', 'lower_is_better', left.latency.totalMs, right.latency.totalMs],
      ['memoryDeltaMb', 'Memory delta', 'mb', 'lower_is_better', left.runtime.memoryDeltaMb, right.runtime.memoryDeltaMb],
    ];
    for (const [key, label, unit, direction, baselineValue, candidateValue] of caseValues) {
      pushMetric(metrics, metric(
        `evaluation.${scope}.${key}`,
        scope,
        label,
        unit,
        direction,
        baselineValue,
        candidateValue,
      ));
    }

    const rightStages = new Map(
      right.transformation.stages.map((stage) => [stage.name, stage]),
    );
    for (const leftStage of left.transformation.stages) {
      const rightStage = rightStages.get(leftStage.name);
      if (!rightStage) continue;
      pushMetric(metrics, metric(
        `evaluation.${scope}.stage:${leftStage.name}.durationUs`,
        `${scope}:stage:${leftStage.name}`,
        'Stage duration',
        'microseconds',
        'lower_is_better',
        leftStage.durationUs,
        rightStage.durationUs,
      ));
    }
  }
  return metrics;
}

export function compareDiagnosticReports(
  baseline: NormalizedDiagnosticReport,
  candidate: NormalizedDiagnosticReport,
): DiagnosticComparison {
  if (baseline.kind !== candidate.kind) {
    const issues: CompatibilityIssue[] = [{
      code: 'report_type_mismatch',
      severity: 'blocker',
      field: 'kind',
      message: 'Benchmark and evaluation reports use different metric semantics.',
    }];
    return {
      status: 'blocked',
      issues,
      deltasAllowed: false,
      recommendationAllowed: false,
      metrics: [],
    };
  }

  let issues: CompatibilityIssue[];
  let metrics: ComparisonMetric[];
  if (baseline.kind === 'benchmark' && candidate.kind === 'benchmark') {
    issues = compareBenchmarkCompatibility(baseline, candidate);
    metrics = issues.some((entry) => entry.severity === 'blocker')
      ? []
      : benchmarkMetrics(baseline, candidate);
  } else if (baseline.kind === 'evaluation' && candidate.kind === 'evaluation') {
    issues = compareEvaluationCompatibility(baseline, candidate);
    metrics = issues.some((entry) => entry.severity === 'blocker')
      ? []
      : evaluationMetrics(baseline, candidate);
  } else {
    issues = [];
    metrics = [];
  }

  const blocked = issues.some((entry) => entry.severity === 'blocker');
  const warned = issues.some((entry) => entry.severity === 'warning');
  return {
    status: blocked ? 'blocked' : warned ? 'warning' : 'compatible',
    issues,
    deltasAllowed: !blocked,
    recommendationAllowed: !blocked && !warned,
    metrics,
  };
}
