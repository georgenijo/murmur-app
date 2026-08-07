import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  UI_LATENCY_SCHEMA_VERSION,
  getUiLatencyBuild,
  type UiLatencySampleV1,
} from '../../lib/uiLatency';
import { LatencyMapView } from './LatencyMapView';

function sample(
  id: string,
  from: string,
  to: string,
  commitMs: number,
  paintedMs: number,
  frameIntervalMs = 16.67,
): UiLatencySampleV1 {
  return {
    schemaVersion: UI_LATENCY_SCHEMA_VERSION,
    sampleId: id,
    from,
    to,
    trigger: 'pointer',
    startedAtMs: Date.now(),
    commitMs,
    firstFrameMs: Math.max(commitMs, paintedMs - frameIntervalMs),
    frameIntervalMs,
    paintedMs,
    build: getUiLatencyBuild(),
  };
}

describe('LatencyMapView', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('shows build-scoped route aggregates and recent transitions', async () => {
    await act(async () => root.render(
      <LatencyMapView samples={[
        sample('1', 'main.history', 'settings.dictation', 4, 18),
        sample('2', 'main.history', 'settings.dictation', 5, 34),
        sample('3', 'settings.dictation', 'settings.model', 3, 12),
      ]} />,
    ));

    expect(container.textContent).toContain('UI latency map');
    expect(container.textContent).toContain('3');
    expect(container.textContent).toContain('2');
    expect(container.textContent).toContain('main.history');
    expect(container.textContent).toContain('settings.dictation');
    expect(container.textContent).toContain('34 ms');
    expect(container.textContent).toContain('Median frame');
    expect(container.textContent).toContain('Median first frame');
    expect(container.textContent).toContain('frame 17 ms');
    expect(container.querySelectorAll('tbody tr')).toHaveLength(2);
  });

  it('explains how to populate an empty build', async () => {
    await act(async () => root.render(<LatencyMapView samples={[]} />));
    expect(container.textContent).toContain('No transitions for this build yet');
    expect(container.textContent).toContain('Move between History, Settings pages, editors, and Diagnostics tabs');
  });
});
