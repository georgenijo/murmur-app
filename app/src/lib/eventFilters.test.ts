import { describe, expect, it } from 'vitest';
import type { AppEvent } from './events';
import {
  matchesCorrelation,
  formatEventForCopy,
  matchesTransformPassId,
  transformPassIdOf,
} from './eventFilters';

function event(data: Record<string, unknown>): AppEvent {
  return {
    timestamp: '2026-07-23T01:02:03.000Z',
    stream: 'transform',
    level: 'info',
    summary: 'transform_pass_outcome',
    data,
  };
}

describe('transform pass event filtering', () => {
  it('matches an exact positive integer pass id only', () => {
    const candidate = event({ transform_pass_id: 42, outcome: 'ready' });
    expect(transformPassIdOf(candidate)).toBe(42);
    expect(matchesTransformPassId(candidate, '42')).toBe(true);
    expect(matchesTransformPassId(candidate, ' 42 ')).toBe(true);
    expect(matchesTransformPassId(candidate, '4')).toBe(false);
    expect(matchesTransformPassId(candidate, '0')).toBe(false);
    expect(matchesTransformPassId(candidate, 'not-an-id')).toBe(false);
  });

  it('copy output retains structured correlation and outcome fields', () => {
    const output = formatEventForCopy(
      event({ transform_pass_id: 42, outcome: 'failed', error_code: 'timeout' }),
    );
    expect(output).toContain('"transform_pass_id":42');
    expect(output).toContain('"error_code":"timeout"');
  });

  it('matches every canonical correlation field exactly', () => {
    const candidate = event({
      run_id: '0123456789abcdef0123456789abcdef',
      recording_id: 17,
      file_run_id: 9,
      transform_pass_id: 42,
    });
    expect(matchesCorrelation(candidate, {
      field: 'run_id',
      value: '0123456789abcdef0123456789abcdef',
    })).toBe(true);
    expect(matchesCorrelation(candidate, {
      field: 'recording_id',
      value: '17',
    })).toBe(true);
    expect(matchesCorrelation(candidate, {
      field: 'file_run_id',
      value: '9',
    })).toBe(true);
    expect(matchesCorrelation(candidate, {
      field: 'recording_id',
      value: '1',
    })).toBe(false);
    expect(matchesCorrelation(candidate, {
      field: 'transform_pass_id',
      value: 'not-an-id',
    })).toBe(false);
  });
});
