import { describe, expect, it } from 'vitest';
import {
  correlatedModelDownloadAttempt,
  modelDownloadLabel,
  modelDownloadPercent,
} from './modelDownload';

describe('model download progress', () => {
  it('reports determinate byte progress for streamed models', () => {
    const progress = { received: 50, total: 200, phase: 'downloading' as const };
    expect(modelDownloadPercent(progress)).toBe(25);
    expect(modelDownloadLabel(progress)).toBe('Downloading...');
  });

  it('keeps Core ML setup indeterminate instead of showing a frozen zero', () => {
    const progress = { received: 0, total: 0, phase: 'installing' as const };
    expect(modelDownloadPercent(progress)).toBeNull();
    expect(modelDownloadLabel(progress)).toBe('Installing...');
  });

  it('names bounded Core ML repair and validation phases', () => {
    expect(modelDownloadLabel({
      received: 0,
      total: 0,
      phase: 'repairing',
      repeatedRepair: false,
    })).toBe('Repairing incomplete install...');
    expect(modelDownloadLabel({
      received: 0,
      total: 0,
      phase: 'repairing',
      repeatedRepair: true,
    })).toBe('Repairing incomplete install again...');
    expect(modelDownloadLabel({ received: 0, total: 0, phase: 'validating' }))
      .toBe('Validating installation...');
  });

  it('treats old unknown-total events as indeterminate', () => {
    expect(modelDownloadPercent({ received: 0, total: 0 })).toBeNull();
  });

  it('clamps malformed byte progress to the completed state', () => {
    expect(modelDownloadPercent({ received: 250, total: 200 })).toBe(100);
  });

  it('correlates progress to one exact model attempt', () => {
    const progress = {
      modelName: 'base.en',
      attemptId: 12,
      received: 10,
      total: 100,
    };
    expect(correlatedModelDownloadAttempt(progress, 'base.en', null)).toBe(12);
    expect(correlatedModelDownloadAttempt(progress, 'base.en', 12)).toBe(12);
    expect(correlatedModelDownloadAttempt(progress, 'base.en', 13)).toBeUndefined();
    expect(correlatedModelDownloadAttempt(progress, 'tiny.en', null)).toBeUndefined();
    expect(correlatedModelDownloadAttempt({ ...progress, attemptId: 0 }, 'base.en', null))
      .toBeUndefined();
  });
});
