import { describe, expect, it } from 'vitest';
import { makeResourceSample, makeRun } from '../../components/log-viewer/testFixtures';
import { mergeResourceSamples, mergeRuns } from './usePerformanceDiagnostics';

describe('performance diagnostics bounded merging', () => {
  it('deduplicates and caps completed runs at 200 newest records', () => {
    const records = Array.from({ length: 205 }, (_, index) => makeRun({
      runId: index.toString(16).padStart(32, '0'),
      startedAtMs: index,
      finishedAtMs: index,
    }));
    const merged = mergeRuns([], records);
    expect(merged).toHaveLength(200);
    expect(merged[0].finishedAtMs).toBe(204);
    expect(merged[merged.length - 1]?.finishedAtMs).toBe(5);

    const replacement = { ...merged[0], appVersion: 'replacement' };
    expect(mergeRuns(merged, [replacement])[0].appVersion).toBe('replacement');
  });

  it('deduplicates and caps resource history at the typed 600-sample window', () => {
    const samples = Array.from({ length: 605 }, (_, index) => makeResourceSample(index));
    const merged = mergeResourceSamples([], samples);
    expect(merged).toHaveLength(600);
    expect(merged[0].observedAtMs).toBe(5);
    expect(merged[merged.length - 1]?.observedAtMs).toBe(604);
  });
});
