import { describe, expect, it } from 'vitest';
import { modelDownloadLabel, modelDownloadPercent } from './modelDownload';

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

  it('treats old unknown-total events as indeterminate', () => {
    expect(modelDownloadPercent({ received: 0, total: 0 })).toBeNull();
  });

  it('clamps malformed byte progress to the completed state', () => {
    expect(modelDownloadPercent({ received: 250, total: 200 })).toBe(100);
  });
});
