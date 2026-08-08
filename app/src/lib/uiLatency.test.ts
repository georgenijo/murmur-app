import { StrictMode, act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  UI_LATENCY_SCHEMA_VERSION,
  beginUiTransition,
  clearUiLatencySamples,
  getUiLatencySamples,
  isUiLatencySampleV1,
  summarizeUiLatency,
  type UiLatencySampleV1,
  useUiLatencyDestination,
} from './uiLatency';

function sample(
  from: string,
  to: string,
  commitMs: number,
  paintedMs: number,
  id: string,
): UiLatencySampleV1 {
  return {
    schemaVersion: UI_LATENCY_SCHEMA_VERSION,
    sampleId: id,
    from,
    to,
    trigger: 'pointer',
    startedAtMs: 1,
    commitMs,
    firstFrameMs: Math.max(commitMs, (commitMs + paintedMs) / 2),
    frameIntervalMs: 2,
    paintedMs,
    build: '0.29.0 · test',
  };
}

describe('UI latency metrics', () => {
  beforeEach(() => {
    localStorage.clear();
    clearUiLatencySamples();
  });

  it('rejects malformed and internally inconsistent samples', () => {
    expect(isUiLatencySampleV1(sample('main', 'settings', 4, 9, 'valid'))).toBe(true);
    expect(isUiLatencySampleV1({
      ...sample('main', 'settings', 10, 5, 'invalid'),
    })).toBe(false);
    expect(isUiLatencySampleV1({
      ...sample('main', 'settings', 4, 9, 'invalid'),
      from: 42,
    })).toBe(false);
    expect(isUiLatencySampleV1({
      ...sample('main', 'settings', 4, 9, 'invalid'),
      frameIntervalMs: -1,
    })).toBe(false);
  });

  it('aggregates exact route edges with median, p95, and maximum paint latency', () => {
    const summary = summarizeUiLatency([
      sample('main.history', 'settings.dictation', 3, 10, '1'),
      sample('main.history', 'settings.dictation', 4, 20, '2'),
      sample('main.history', 'settings.dictation', 5, 80, '3'),
      sample('settings.dictation', 'settings.model', 2, 8, '4'),
    ]);

    expect(summary).toEqual([
      {
        from: 'main.history',
        to: 'settings.dictation',
        count: 3,
        medianCommitMs: 4,
        medianFirstFrameMs: 12,
        p95FirstFrameMs: 42.5,
        medianFrameCount: 4,
        medianPaintedMs: 20,
        p95PaintedMs: 80,
        maxPaintedMs: 80,
      },
      {
        from: 'settings.dictation',
        to: 'settings.model',
        count: 1,
        medianCommitMs: 2,
        medianFirstFrameMs: 5,
        p95FirstFrameMs: 5,
        medianFrameCount: 2,
        medianPaintedMs: 8,
        p95PaintedMs: 8,
        maxPaintedMs: 8,
      },
    ]);
  });

  it('survives StrictMode effect replay and records one painted transition', async () => {
    let nextFrame = 1;
    const frames = new Map<number, FrameRequestCallback>();
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
      const id = nextFrame++;
      frames.set(id, callback);
      return id;
    });
    vi.stubGlobal('cancelAnimationFrame', (id: number) => {
      frames.delete(id);
    });

    function Destination() {
      useUiLatencyDestination('settings.dictation');
      return null;
    }

    const container = document.createElement('div');
    const root = createRoot(container);
    beginUiTransition('main.history', 'settings.dictation', 'pointer');
    await act(async () => {
      root.render(createElement(StrictMode, null, createElement(Destination)));
    });

    const firstFrameTimestamp = performance.now() + 8;
    for (let pass = 0; pass < 2; pass += 1) {
      const pendingFrames = Array.from(frames.entries());
      frames.clear();
      await act(async () => {
        for (const [, callback] of pendingFrames) callback(firstFrameTimestamp + pass * 16.67);
      });
    }

    expect(getUiLatencySamples()).toHaveLength(1);
    expect(getUiLatencySamples()[0]).toMatchObject({
      from: 'main.history',
      to: 'settings.dictation',
      trigger: 'pointer',
    });
    expect(getUiLatencySamples()[0].frameIntervalMs).toBeCloseTo(16.67, 2);
    await act(async () => root.unmount());
    vi.unstubAllGlobals();
  });
});
