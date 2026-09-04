import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HomeRecordingBar } from './HomeRecordingBar';
import { HomeSidebar } from './HomeSidebar';
import { buildHistoryActivity } from './HomeInsightsRail';
import { PersonalizationCard } from './PersonalizationCard';

describe('home dashboard interactions', () => {
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

  it('preserves recording, stop, and file-transcription actions', async () => {
    const onRecord = vi.fn();
    const onStop = vi.fn();
    const onTranscribeFile = vi.fn();
    const renderBar = async (status: 'idle' | 'recording') => {
      await act(async () => root.render(
        <HomeRecordingBar
          status={status}
          initialized
          recordingDuration={12}
          audioLevel={0.04}
          triggerKey="shift_l"
          recordingMode="hold_down"
          meetingPhase="idle"
          onRecord={onRecord}
          onStop={onStop}
          onTranscribeFile={onTranscribeFile}
        />,
      ));
    };

    await renderBar('idle');
    await act(async () => (container.querySelector('[data-testid="home-record-button"]') as HTMLButtonElement).click());
    await act(async () => Array.from(container.querySelectorAll('button')).find((button) => button.textContent?.includes('Transcribe File'))?.click());
    expect(onRecord).toHaveBeenCalledOnce();
    expect(onTranscribeFile).toHaveBeenCalledOnce();

    await renderBar('recording');
    const record = container.querySelector('[data-testid="home-record-button"]') as HTMLButtonElement;
    expect(record.getAttribute('aria-label')).toBe('Stop recording, 0:12');
    await act(async () => record.click());
    expect(onStop).toHaveBeenCalledOnce();
    expect(container.querySelectorAll('.home-record-waveform span')).toHaveLength(5);
  });

  it('keeps the sidebar focused on the four primary destinations', async () => {
    const onNavigate = vi.fn();
    await act(async () => root.render(
      <HomeSidebar active="home" onNavigate={onNavigate} />,
    ));

    const button = (label: string) => Array.from(container.querySelectorAll('button'))
      .find((candidate) => candidate.getAttribute('aria-label') === label) as HTMLButtonElement;
    await act(async () => button('Insights').click());

    expect(onNavigate).toHaveBeenCalledWith('insights');
    expect(Array.from(container.querySelectorAll('nav button')).map((item) => item.getAttribute('aria-label'))).toEqual([
      'Home', 'Notetaker', 'Queries', 'Insights',
    ]);
    expect(container.textContent).not.toContain('Customize');
    expect(container.querySelector('.home-sidebar-bottom')?.textContent).toContain('Everything stays on this Mac.');
  });

  it('describes auditable milestones rather than an opaque score', async () => {
    await act(async () => root.render(
      <PersonalizationCard
        summary={{
          stage: 'Developing',
          completed: 1,
          total: 3,
          milestones: [
            { id: 'vocabulary', label: 'Preferred terms', detail: '2 enabled', complete: true },
            { id: 'styles', label: 'App styles', detail: 'Choose a style for an app', complete: false },
            { id: 'usage', label: 'Regular use', detail: '2 of 5 active days', complete: false },
          ],
          nextAction: 'Configure an app style.',
        }}
        expanded
        onOpenVocabulary={vi.fn()}
        onOpenStyles={vi.fn()}
      />,
    ));

    expect(container.textContent).toContain('Developing');
    expect(container.textContent).toContain('1 of 3 set up');
    expect(container.textContent).toContain('not a voice-training or confidence score');
    expect(container.textContent).not.toContain('%');
  });

  it('builds activity from 14 local history days and hides thinner data', () => {
    const entries = Array.from({ length: 14 }, (_, index) => ({
      id: `entry-${index}`,
      text: `dictation ${index}`,
      timestamp: new Date(2026, 7, 15 + index, 23, 30).getTime(),
      duration: 1,
      source: 'recording' as const,
    }));
    const referenceDate = new Date(2026, 8, 4, 12);

    expect(buildHistoryActivity(entries.slice(0, 13), referenceDate)).toBeNull();
    const activity = buildHistoryActivity(entries, referenceDate);
    expect(activity?.data).toHaveLength(14);
    expect(activity?.data[0]).toMatchObject({ date: '2026-08-15', value: 1 });
    expect(activity?.startDate).toBe('2026-07-11');
    expect(activity?.endDate).toBe('2026-09-04');
  });
});
