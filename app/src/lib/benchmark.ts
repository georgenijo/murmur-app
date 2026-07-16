import { invoke } from '@tauri-apps/api/core';

export type BenchmarkPreset = 'quick' | 'standard' | 'thorough';

export interface BenchmarkModel {
  modelName: string;
  label: string;
  backend: string;
  accelerator: string;
  size: string;
  supported: boolean;
  installed: boolean;
}

export interface BenchmarkProgress {
  completed: number;
  total: number;
  modelName: string;
  modelLabel: string;
  fixture: string | null;
  phase: 'loading' | 'warming' | 'measuring' | 'complete';
}

export interface BenchmarkFixtureResult {
  fixtureId: string;
  label: string;
  audioSeconds: number;
  warmMedianMs: number;
  warmP95Ms: number;
  realtimeFactor: number;
  wordErrorRate: number;
  wordErrors: number;
  referenceWords: number;
  reference: string;
  transcript: string;
}

export interface BenchmarkModelResult {
  modelName: string;
  label: string;
  backend: string;
  accelerator: string;
  modelLoadMs: number | null;
  firstInferenceMs: number | null;
  warmMedianMs: number | null;
  warmP95Ms: number | null;
  realtimeFactor: number | null;
  wordErrorRate: number | null;
  memoryDeltaMb: number;
  fixtures: BenchmarkFixtureResult[];
  error: string | null;
}

export interface BenchmarkReport {
  createdAt: string;
  appVersion: string;
  platform: string;
  preset: BenchmarkPreset;
  iterations: number;
  results: BenchmarkModelResult[];
  recommendations: {
    fastest: string | null;
    mostAccurate: string | null;
    balanced: string | null;
  };
}

export const MAX_SAVED_BENCHMARK_REPORTS = 10;

export function addBenchmarkReport(
  reports: BenchmarkReport[],
  next: BenchmarkReport,
): BenchmarkReport[] {
  return [
    next,
    ...reports.filter((report) => report.createdAt !== next.createdAt),
  ].slice(0, MAX_SAVED_BENCHMARK_REPORTS);
}

export function getBenchmarkModels(): Promise<BenchmarkModel[]> {
  return invoke('get_benchmark_models');
}

export function runBenchmark(modelNames: string[], preset: BenchmarkPreset): Promise<BenchmarkReport> {
  return invoke('run_benchmark', { request: { modelNames, preset } });
}

export function cancelBenchmark(): Promise<boolean> {
  return invoke('cancel_benchmark');
}
