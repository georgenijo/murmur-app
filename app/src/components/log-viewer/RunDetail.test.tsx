import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ShadowMeasurementV1 } from '../../lib/performance';
import { makeRun } from './testFixtures';
import { RunDetail } from './RunDetail';

const shadow: ShadowMeasurementV1 = {
  schemaVersion: 1, pressSample: 16000, pauseSample: null,
  trailingSilenceBeforePressMs: 0, trailingSilenceBeforeStopMs: 500,
  pressInputMatches: false, pauseInputMatches: null, previewHeldBackendAtPress: false,
  determinism: 'skippedForegroundLatencyRisk', decodeMatches: null,
};
describe('RunDetail shadow evidence', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    container = document.createElement('div'); document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  async function render(value?: ShadowMeasurementV1 | null) {
    await act(async () => root.render(<RunDetail run={makeRun({ shadow: value })} onBack={vi.fn()} onShowEvents={vi.fn()} />));
  }
  function value(label: string) {
    const term = Array.from(container.querySelectorAll('dt')).find(node => node.textContent === label);
    return term?.nextElementSibling?.textContent;
  }
  it('renders measured false, zero, unavailable and skipped distinctly', async () => {
    await render(shadow);
    expect(value('Press input matches final')).toBe('No');
    expect(value('Pause input matches final')).toBe('Unavailable');
    expect(value('Preview held backend at press')).toBe('No');
    expect(value('Silence before press')).toBe('0 ms');
    expect(value('Silence before stop')).toBe('500 ms');
    expect(value('Press sample')).toBe('16000');
    expect(value('Pause sample')).toBe('Unavailable');
    expect(value('Repeat decode')).toBe('Skipped: foreground latency risk');
    expect(value('Decode outputs match')).toBe('Not compared');
  });
  it('labels historical absent and null companions without inventing data', async () => {
    for (const value of [undefined, null]) {
      await render(value);
      expect(container.textContent).toContain('Shadow evidence not recorded for this run.');
      expect(container.querySelector('dt')).toBeNull();
    }
  });
  it('renders older comparison results and preserves false', async () => {
    await render({ ...shadow, determinism: 'compared', decodeMatches: false, pressInputMatches: true });
    expect(value('Repeat decode')).toBe('Compared');
    expect(value('Decode outputs match')).toBe('No');
    expect(value('Press input matches final')).toBe('Yes');
  });
  it.each([
    ['disabled', 'Disabled'], ['notAttempted', 'Not attempted'],
    ['backendUnavailable', 'Skipped: backend unavailable'], ['superseded', 'Skipped: recording superseded'],
    ['decodeFailed', 'Unavailable: decode failed'], ['unsupportedBackend', 'Not applicable to this backend'],
    ['pending', 'Pending'],
  ] as const)('renders %s as %s', async (determinism, label) => {
    await render({ ...shadow, determinism });
    expect(value('Repeat decode')).toBe(label);
    expect(value('Decode outputs match')).toBe('Not compared');
  });
});
