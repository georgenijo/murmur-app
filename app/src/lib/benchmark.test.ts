import { describe, expect, it } from 'vitest';
import {
  BenchmarkReport,
  MAX_SAVED_BENCHMARK_REPORTS,
  addBenchmarkReport,
  clearBenchmarkReports,
  loadBenchmarkReports,
  saveBenchmarkReports,
} from './benchmark';

function report(createdAt: string): BenchmarkReport {
  return {
    createdAt,
    appVersion: '0.16.0',
    platform: 'macos aarch64',
    preset: 'quick',
    iterations: 3,
    sharedInitMs: 1200,
    results: [],
    recommendations: { fastest: null, mostAccurate: null, balanced: null },
  };
}

describe('addBenchmarkReport', () => {
  it('puts the newest report first and deduplicates the same run', () => {
    const older = report('2026-07-16T12:00:00Z');
    const newest = report('2026-07-16T13:00:00Z');
    expect(addBenchmarkReport([older, newest], newest)).toEqual([newest, older]);
  });

  it('keeps only the latest ten reports', () => {
    const existing = Array.from(
      { length: MAX_SAVED_BENCHMARK_REPORTS },
      (_, index) => report(`2026-07-16T${String(index).padStart(2, '0')}:00:00Z`),
    );
    const newest = report('2026-07-17T00:00:00Z');
    const result = addBenchmarkReport(existing, newest);
    expect(result).toHaveLength(MAX_SAVED_BENCHMARK_REPORTS);
    expect(result[0]).toBe(newest);
    expect(result).not.toContain(existing[0]);
  });
});

describe('benchmark report storage', () => {
  it('migrates a valid legacy report and clears the legacy key', () => {
    const legacy = report('2026-07-16T12:00:00Z');
    localStorage.setItem('murmur-benchmark-report', JSON.stringify(legacy));

    expect(loadBenchmarkReports()).toEqual([legacy]);
    expect(localStorage.getItem('murmur-benchmark-report')).toBeNull();
    expect(JSON.parse(localStorage.getItem('murmur-benchmark-reports') ?? '[]')).toEqual([legacy]);
  });

  it('rejects malformed reports and clears both keys', () => {
    localStorage.setItem('murmur-benchmark-reports', JSON.stringify([report('2026-07-16T12:00:00Z'), { createdAt: 'bad' }]));
    expect(loadBenchmarkReports()).toEqual([report('2026-07-16T12:00:00Z')]);

    saveBenchmarkReports([report('2026-07-16T13:00:00Z')]);
    clearBenchmarkReports();
    expect(localStorage.getItem('murmur-benchmark-reports')).toBeNull();
    expect(localStorage.getItem('murmur-benchmark-report')).toBeNull();
  });
});
