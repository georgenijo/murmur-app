import { describe, expect, it } from 'vitest';
import { makeRun, measured, unavailable } from '../components/log-viewer/testFixtures';
import type { PerformanceRunV1 } from './performance';
import { summarizeColdModelLoads } from './coldModelLoads';

function dictation(
  warmState: 'warm' | 'coldLoaded' | 'unknown',
  loadMs?: number,
  overrides: Partial<PerformanceRunV1> = {},
): PerformanceRunV1 {
  const base = makeRun();
  return makeRun({
    runtimes: [{ ...base.runtimes[0], warmState }],
    stages: base.stages.map(stage => stage.stage === 'modelLoad'
      ? { ...stage, durationMs: loadMs === undefined ? unavailable() : measured(loadMs), outcome: 'completed' }
      : stage),
    ...overrides,
  });
}

describe('summarizeColdModelLoads', () => {
  it('counts cold loads among successful dictations with a known warm state', () => {
    const summary = summarizeColdModelLoads([
      dictation('warm', 0),
      dictation('warm', 0),
      dictation('coldLoaded', 300),
      dictation('coldLoaded', 100),
      dictation('coldLoaded'),
      dictation('unknown'),
      dictation('coldLoaded', 900, { outcome: { status: 'noSpeech' } }),
      dictation('coldLoaded', 900, { kind: 'fileTranscription', correlation: { kind: 'fileTranscription', fileRunId: 1 } }),
    ]);
    expect(summary).toEqual({ dictations: 5, coldLoaded: 3, medianColdLoadMs: 200 });
  });

  it('reports no median without measured cold loads', () => {
    expect(summarizeColdModelLoads([])).toEqual({ dictations: 0, coldLoaded: 0, medianColdLoadMs: null });
    expect(summarizeColdModelLoads([dictation('warm', 0)]).medianColdLoadMs).toBeNull();
  });
});
