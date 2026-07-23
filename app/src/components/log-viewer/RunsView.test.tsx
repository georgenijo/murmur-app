import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CorrelationFilter } from '../../lib/eventFilters';
import type { PerformanceRunV1 } from '../../lib/performance';
import { makeRun, measured, notApplicable } from './testFixtures';

const mocks = vi.hoisted(() => ({
  getPerformanceRun: vi.fn(),
}));

vi.mock('../../lib/performance', async importOriginal => ({
  ...(await importOriginal<typeof import('../../lib/performance')>()),
  getPerformanceRun: mocks.getPerformanceRun,
}));

import { RunsView } from './RunsView';

function runs(): PerformanceRunV1[] {
  const dictation = makeRun();
  const file = makeRun({
    runId: '1123456789abcdef0123456789abcdef',
    kind: 'fileTranscription',
    correlation: { kind: 'fileTranscription', fileRunId: 9 },
    outcome: { status: 'noSpeech' },
  });
  const transformBase = makeRun();
  const transform = makeRun({
    runId: '2123456789abcdef0123456789abcdef',
    kind: 'selectedTextTransform',
    correlation: { kind: 'selectedTextTransform', transformPassId: 42 },
    outcome: { status: 'failed', stage: 'generation', errorCode: 'transformStageFailed' },
    input: {
      audioDurationMs: notApplicable(),
      inputSizeBucket: measured('small'),
      outputSizeBucket: measured('small'),
      outputTokenCount: measured(20),
    },
    stages: transformBase.stages.map(stage => {
      if (stage.stage === 'selectedTextCapture') {
        return { ...stage, durationMs: measured(12), outcome: 'completed' };
      }
      if (stage.stage === 'generation') {
        return { ...stage, durationMs: measured(500), outcome: 'failed' };
      }
      return stage;
    }),
  });
  return [transform, file, dictation];
}

describe('RunsView', () => {
  let container: HTMLDivElement;
  let root: Root;
  let onClear: ReturnType<typeof vi.fn<() => Promise<void>>>;
  let onShowEvents: ReturnType<typeof vi.fn<(filter: CorrelationFilter) => void>>;

  beforeEach(() => {
    vi.clearAllMocks();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    onClear = vi.fn<() => Promise<void>>().mockResolvedValue(undefined);
    onShowEvents = vi.fn<(filter: CorrelationFilter) => void>();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  async function renderRuns(items = runs()) {
    await act(async () => {
      root.render(
        <RunsView
          runs={items}
          loading={false}
          error={null}
          cleared={false}
          clearing={false}
          clearError={null}
          onRetry={vi.fn()}
          onClear={onClear}
          onShowEvents={onShowEvents}
        />,
      );
    });
  }

  it('filters first-class run kinds and preserves every terminal outcome', async () => {
    await renderRuns();
    expect(container.textContent).toContain('Dictation');
    expect(container.textContent).toContain('File transcription');
    expect(container.textContent).toContain('Selected-text transform');
    expect(container.textContent).toContain('Success');
    expect(container.textContent).toContain('No speech');
    expect(container.textContent).toContain('Failed');

    const kindSelect = container.querySelectorAll('select')[0] as HTMLSelectElement;
    await act(async () => {
      kindSelect.value = 'selectedTextTransform';
      kindSelect.dispatchEvent(new Event('change', { bubbles: true }));
    });
    const tableBodyText = container.querySelector('tbody')?.textContent ?? '';
    expect(tableBodyText).toContain('Selected-text transform');
    expect(tableBodyText).not.toContain('File transcription');
    expect(tableBodyText).not.toContain('Dictation');
  });

  it('opens typed detail, renders ordered durations without offsets, and jumps to Events', async () => {
    const transform = runs()[0];
    mocks.getPerformanceRun.mockResolvedValue(transform);
    await renderRuns([transform]);
    const detailButton = container.querySelector('button[aria-label^="View details"]') as HTMLButtonElement;
    await act(async () => {
      detailButton.click();
      await Promise.resolve();
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(container.textContent).toContain('Phase waterfall');
    expect(container.textContent).toContain('V1 does not record absolute offsets');
    expect(container.textContent).toContain('12 ms');
    expect(container.textContent).toContain('500 ms');
    expect(container.textContent).toContain('Not applicable');
    const waterfallText = container.querySelector('ol')?.textContent ?? '';
    expect(waterfallText.indexOf('Selected-text capture'))
      .toBeLessThan(waterfallText.indexOf('Generation'));
    const eventButton = Array.from(container.querySelectorAll('button'))
      .find(button => button.textContent === 'Show correlated Events')!;
    await act(async () => eventButton.click());
    expect(onShowEvents).toHaveBeenCalledWith({
      field: 'transform_pass_id',
      value: '42',
    });
  });

  it('uses an exact-scope confirmation before clearing performance data', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    await renderRuns([makeRun()]);
    const clearButton = Array.from(container.querySelectorAll('button'))
      .find(button => button.textContent === 'Clear Performance Data')!;
    await act(async () => {
      clearButton.click();
      await Promise.resolve();
    });
    expect(window.confirm).toHaveBeenCalledWith(expect.stringContaining(
      'This does not remove Events, logs, transcription history, settings, knowledge, or benchmark reports.',
    ));
    expect(onClear).toHaveBeenCalledOnce();
  });

  it('distinguishes loading, cleared, filtered-empty, and error states', async () => {
    await act(async () => {
      root.render(
        <RunsView
          runs={[]}
          loading
          error={null}
          cleared={false}
          clearing={false}
          clearError={null}
          onRetry={vi.fn()}
          onClear={onClear}
          onShowEvents={onShowEvents}
        />,
      );
    });
    expect(container.querySelector('[aria-label="Loading performance runs"]')).not.toBeNull();

    await act(async () => {
      root.render(
        <RunsView
          runs={[]}
          loading={false}
          error={null}
          cleared
          clearing={false}
          clearError={null}
          onRetry={vi.fn()}
          onClear={onClear}
          onShowEvents={onShowEvents}
        />,
      );
    });
    expect(container.textContent).toContain('Performance data was cleared');

    await act(async () => {
      root.render(
        <RunsView
          runs={[]}
          loading={false}
          error="store unavailable"
          cleared={false}
          clearing={false}
          clearError={null}
          onRetry={vi.fn()}
          onClear={onClear}
          onShowEvents={onShowEvents}
        />,
      );
    });
    expect(container.textContent).toContain('Run history unavailable');
    expect(container.querySelector('[role="alert"]')).not.toBeNull();
  });
});
