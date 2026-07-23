import { describe, expect, it } from 'vitest';
import benchmarkLegacy from './__fixtures__/diagnostic-reports/benchmark-legacy.json';
import benchmarkV2 from './__fixtures__/diagnostic-reports/benchmark-v2.json';
import evaluationDeterministic from './__fixtures__/diagnostic-reports/evaluation-v1-deterministic.json';
import evaluationHardware from './__fixtures__/diagnostic-reports/evaluation-v1-hardware.json';
import {
  DIAGNOSTIC_REPORT_LIMITS,
  MAX_DIAGNOSTIC_REPORT_BYTES,
  normalizeLocalBenchmarkReport,
  parseDiagnosticReportJson,
} from './diagnosticReports';
import type { BenchmarkReport } from './benchmark';

function json(value: unknown): string {
  return JSON.stringify(value);
}

function parse(value: unknown) {
  const contents = json(value);
  return parseDiagnosticReportJson(contents, new TextEncoder().encode(contents).byteLength);
}

describe('parseDiagnosticReportJson', () => {
  it('imports benchmark legacy and v2 fixtures with explicit schema labels', () => {
    const legacy = parse(benchmarkLegacy);
    const versioned = parse(benchmarkV2);

    expect(legacy.ok).toBe(true);
    expect(versioned.ok).toBe(true);
    if (!legacy.ok || !versioned.ok) return;
    expect(legacy.report).toMatchObject({
      kind: 'benchmark',
      source: 'imported',
      schemaVersion: 'legacy',
    });
    expect(legacy.report.privacyWarnings).toContain(
      'Legacy benchmark reports omit environment, corpus, and execution configuration metadata.',
    );
    expect(versioned.report).toMatchObject({
      kind: 'benchmark',
      source: 'imported',
      schemaVersion: 2,
    });
  });

  it('imports deterministic and hardware EvaluationReportV1 fixtures', () => {
    const deterministic = parse(evaluationDeterministic);
    const hardware = parse(evaluationHardware);

    expect(deterministic.ok).toBe(true);
    expect(hardware.ok).toBe(true);
    if (!deterministic.ok || !hardware.ok) return;
    expect(deterministic.report).toMatchObject({
      kind: 'evaluation',
      schemaVersion: 1,
      fixtureVersion: 1,
      tier: 'deterministic',
    });
    expect(hardware.report).toMatchObject({
      kind: 'evaluation',
      schemaVersion: 1,
      fixtureVersion: 1,
      tier: 'hardware',
    });
    expect(deterministic.report.privacyWarnings[0]).toContain(
      'curated fixture transcripts and per-stage text',
    );
  });

  it('drops report text, evaluator failures, fixture context, and audio paths', () => {
    const reports = [parse(benchmarkV2), parse(evaluationDeterministic), parse(evaluationHardware)];
    for (const result of reports) {
      expect(result.ok).toBe(true);
      if (!result.ok) continue;
      const normalized = JSON.stringify(result.report);
      expect(normalized).not.toContain('PRIVATE_');
      expect(normalized).not.toContain('expectedRaw');
      expect(normalized).not.toContain('actualRaw');
      expect(normalized).not.toContain('audioPath');
      expect(normalized).not.toContain('bundleId');
      expect(normalized).not.toContain('matchedProfile');
      expect(normalized).not.toContain('failures');
    }
  });

  it('returns stable content-free errors for empty, malformed, and mismatched input', () => {
    const empty = parseDiagnosticReportJson(' \n ', 3);
    const malformed = parseDiagnosticReportJson('{"secret":"PRIVATE_PARSE_SENTINEL"', 35);
    const mismatch = parse({ createdAt: 'PRIVATE_SCHEMA_SENTINEL', results: [] });

    expect(empty).toEqual({
      ok: false,
      error: { code: 'empty', message: 'The selected diagnostic report is empty.' },
    });
    expect(malformed).toEqual({
      ok: false,
      error: { code: 'malformed_json', message: 'The selected file is not valid JSON.' },
    });
    expect(mismatch).toMatchObject({ ok: false, error: { code: 'schema_mismatch' } });
    expect(JSON.stringify([empty, malformed, mismatch])).not.toContain('PRIVATE_');
  });

  it('rejects declared or actual input larger than the 8 MiB cap before parsing', () => {
    expect(parseDiagnosticReportJson('{}', MAX_DIAGNOSTIC_REPORT_BYTES + 1)).toEqual({
      ok: false,
      error: { code: 'oversized', message: 'Diagnostic reports are limited to 8 MiB.' },
    });
    const oversized = `"${'x'.repeat(MAX_DIAGNOSTIC_REPORT_BYTES)}"`;
    expect(parseDiagnosticReportJson(oversized, 1)).toMatchObject({
      ok: false,
      error: { code: 'oversized' },
    });
  });

  it('rejects unknown benchmark, report, and fixture versions', () => {
    expect(parse({ ...benchmarkV2, reportVersion: 3 })).toMatchObject({
      ok: false,
      error: { code: 'unsupported_version' },
    });
    expect(parse({ ...evaluationDeterministic, reportVersion: 2 })).toMatchObject({
      ok: false,
      error: { code: 'unsupported_version' },
    });
    expect(parse({ ...evaluationDeterministic, fixtureVersion: 2 })).toMatchObject({
      ok: false,
      error: { code: 'unsupported_version' },
    });
  });

  it('rejects unknown fields, duplicate IDs, and inconsistent summaries', () => {
    expect(parse({ ...benchmarkV2, unexpected: true })).toMatchObject({
      ok: false,
      error: { code: 'schema_mismatch' },
    });
    const duplicateCase = structuredClone(evaluationDeterministic);
    duplicateCase.cases.push(structuredClone(duplicateCase.cases[0]));
    duplicateCase.summary.total = 2;
    duplicateCase.summary.passed = 2;
    expect(parse(duplicateCase)).toMatchObject({
      ok: false,
      error: { code: 'schema_mismatch' },
    });
    expect(parse({
      ...evaluationDeterministic,
      summary: { ...evaluationDeterministic.summary, total: 2 },
    })).toMatchObject({
      ok: false,
      error: { code: 'schema_mismatch' },
    });
  });

  it('enforces model, fixture, case, and stage collection bounds', () => {
    const tooManyModels = structuredClone(benchmarkV2);
    tooManyModels.results = Array.from(
      { length: DIAGNOSTIC_REPORT_LIMITS.benchmarkModels + 1 },
      (_, index) => ({
        ...structuredClone(benchmarkV2.results[0]),
        modelName: `model-${index}`,
      }),
    );
    expect(parse(tooManyModels)).toMatchObject({
      ok: false,
      error: { code: 'collection_limit' },
    });

    const tooManyFixtures = structuredClone(benchmarkV2);
    tooManyFixtures.results[0].fixtures = Array.from(
      { length: DIAGNOSTIC_REPORT_LIMITS.benchmarkFixturesPerModel + 1 },
      () => ({} as typeof benchmarkV2.results[0]['fixtures'][number]),
    );
    expect(parse(tooManyFixtures)).toMatchObject({
      ok: false,
      error: { code: 'collection_limit' },
    });

    const tooManyCases = structuredClone(evaluationDeterministic);
    tooManyCases.cases = Array.from(
      { length: DIAGNOSTIC_REPORT_LIMITS.evaluationCases + 1 },
      () => ({} as typeof evaluationDeterministic.cases[number]),
    );
    expect(parse(tooManyCases)).toMatchObject({
      ok: false,
      error: { code: 'collection_limit' },
    });

    const tooManyStages = structuredClone(evaluationDeterministic);
    tooManyStages.cases[0].transformation.stages = Array.from(
      { length: DIAGNOSTIC_REPORT_LIMITS.evaluationStagesPerCase + 1 },
      (_, index) => ({
        ...structuredClone(evaluationDeterministic.cases[0].transformation.stages[0]),
        name: `stage-${index}`,
      }),
    );
    expect(parse(tooManyStages)).toMatchObject({
      ok: false,
      error: { code: 'collection_limit' },
    });
  });

  it('normalizes an already validated local benchmark without changing storage', () => {
    const result = normalizeLocalBenchmarkReport(benchmarkV2 as BenchmarkReport);
    expect(result).toMatchObject({
      ok: true,
      report: { kind: 'benchmark', source: 'local', schemaVersion: 2 },
    });
  });
});
