import { beforeEach, describe, expect, it } from 'vitest';
import {
  BenchmarkReport,
  MAX_SAVED_BENCHMARK_REPORTS,
  addBenchmarkReport,
  benchmarkReportFileName,
  clearBenchmarkReports,
  loadBenchmarkReports,
  saveBenchmarkReports,
} from './benchmark';

beforeEach(() => localStorage.clear());

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

function versionedReport(createdAt: string): BenchmarkReport {
  return {
    ...report(createdAt),
    reportVersion: 2,
    environment: {
      os: 'macOS', osVersion: '15.5', architecture: 'aarch64',
      hardwareModel: 'Mac14,3', chip: 'Apple M2', memoryMb: 16384,
    },
    corpus: {
      language: 'en', fixtureIds: ['short', 'medium'], fixtureCount: 2,
      referenceWords: 29, provenance: 'Bundled macOS Samantha TTS fixtures',
      limitation: 'Directional local comparison only.',
    },
    configuration: {
      vadThreshold: 0.5,
      executionPath: 'full-buffer final transcription after recording stops',
      transcriptTransformProfile: 'default local delivery pipeline',
      percentileMethod: 'nearest-rank over measured warm iterations',
      modelRunOrder: ['tiny.en'],
      sharedInitOrder: ['tiny.en'],
    },
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

describe('benchmarkReportFileName', () => {
  it('uses the chip label and sanitizes the timestamp for versioned reports', () => {
    const name = benchmarkReportFileName(versionedReport('2026-07-20T14:30:00.000Z'));
    expect(name).toBe('benchmark-0-16-0-Apple-M2-2026-07-20T14-30-00-000Z.json');
  });

  it('falls back to the platform for legacy reports without environment', () => {
    expect(benchmarkReportFileName(report('2026-07-16T09:00:00Z')))
      .toBe('benchmark-0-16-0-macos-aarch64-2026-07-16T09-00-00Z.json');
  });
});

describe('benchmark report storage', () => {
  it('round-trips versioned trustworthy-state metadata', () => {
    const current = versionedReport('2026-07-20T12:00:00Z');
    saveBenchmarkReports([current]);
    expect(loadBenchmarkReports()).toEqual([current]);
  });

  it('keeps pre-metadata reports readable after the additive schema change', () => {
    const previous = report('2026-07-19T12:00:00Z');
    localStorage.setItem('murmur-benchmark-reports', JSON.stringify([previous]));
    expect(loadBenchmarkReports()).toEqual([previous]);
  });

  it('rejects incomplete or unknown versioned metadata instead of presenting it as trustworthy', () => {
    const incomplete = { ...versionedReport('2026-07-20T12:00:00Z'), configuration: undefined };
    const unknown = { ...versionedReport('2026-07-20T13:00:00Z'), reportVersion: 3 };
    localStorage.setItem('murmur-benchmark-reports', JSON.stringify([incomplete, unknown]));
    expect(loadBenchmarkReports()).toEqual([]);
  });

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
