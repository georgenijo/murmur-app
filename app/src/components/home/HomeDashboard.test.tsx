import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HomeRecordingBar } from './HomeRecordingBar';
import { HomeSidebar } from './HomeSidebar';
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

  it('keeps the talk card to one action and a short hint', async () => {
    const onRecord = vi.fn();
    const onStop = vi.fn();
    const renderBar = async (
      status: 'idle' | 'recording' | 'processing' | 'recovering',
      recordingMode: 'hold_down' | 'double_tap' | 'both' = 'hold_down',
    ) => {
      await act(async () => root.render(
        <HomeRecordingBar
          status={status}
          initialized
          recordingDuration={12}
          audioLevel={0.04}
          triggerKey="shift_l"
          recordingMode={recordingMode}
          meetingPhase="idle"
          onRecord={onRecord}
          onStop={onStop}
        />,
      ));
    };

    await renderBar('idle');
    expect(container.querySelectorAll('button')).toHaveLength(1);
    expect(container.textContent).toContain('Click to start talking');
    expect(container.textContent).toContain('or hold ⇧ Left Shift in any app');
    const startLabel = container.querySelector<HTMLElement>('.home-talk-title strong');
    expect(startLabel?.closest('button')?.dataset.testid).toBe('home-record-button');
    await act(async () => startLabel?.click());
    expect(onRecord).toHaveBeenCalledOnce();

    await renderBar('idle', 'double_tap');
    expect(container.textContent).toContain('or double-tap ⇧ Left Shift in any app');
    await renderBar('idle', 'both');
    expect(container.textContent).toContain('or hold or double-tap ⇧ Left Shift in any app');

    await renderBar('recording');
    const record = container.querySelector('[data-testid="home-record-button"]') as HTMLButtonElement;
    expect(record.getAttribute('aria-label')).toBe('Click to stop recording, 0:12');
    expect(container.textContent).toContain('Listening · 0:12');
    await act(async () => container.querySelector<HTMLElement>('.home-talk-hint')?.click());
    expect(onStop).toHaveBeenCalledOnce();
    expect(container.querySelectorAll('.home-record-waveform span')).toHaveLength(5);

    await renderBar('recovering');
    expect(container.textContent).toContain('Reconnecting your microphone…');
    expect((container.querySelector('[data-testid="home-record-button"]') as HTMLButtonElement).disabled).toBe(true);
  });

  it.each([
    ['current', 'Current text field'],
    ['recent', 'Recent text field'],
    ['unavailable', 'Clipboard only'],
  ] as const)('shows the %s smart destination while recording and processing', async (smartDestinationStatus, label) => {
    const renderBar = async (status: 'recording' | 'processing' | 'idle') => {
      await act(async () => root.render(
        <HomeRecordingBar
          status={status}
          initialized
          recordingDuration={12}
          audioLevel={0.04}
          smartDestinationStatus={smartDestinationStatus}
          triggerKey="shift_l"
          recordingMode="hold_down"
          meetingPhase="idle"
          onRecord={vi.fn()}
          onStop={vi.fn()}
        />,
      ));
    };

    await renderBar('recording');
    expect(container.querySelector('.home-talk-destination')?.textContent).toBe(label);
    expect(container.querySelector('.home-talk-destination')?.getAttribute('aria-label'))
      .toBe(`Destination: ${label}`);

    await renderBar('processing');
    expect(container.querySelector('.home-talk-destination')?.textContent).toBe(label);

    await renderBar('idle');
    expect(container.querySelector('.home-talk-destination')).toBeNull();
  });

  it('names a verified destination app without naming clipboard-only delivery', async () => {
    const renderBar = async (smartDestinationStatus: 'recent' | 'unavailable') => {
      await act(async () => root.render(
        <HomeRecordingBar
          status="recording"
          initialized
          recordingDuration={12}
          audioLevel={0.04}
          smartDestinationStatus={smartDestinationStatus}
          smartDestinationAppName="T3 Code"
          triggerKey="shift_l"
          recordingMode="hold_down"
          meetingPhase="idle"
          onRecord={vi.fn()}
          onStop={vi.fn()}
        />,
      ));
    };

    await renderBar('recent');
    expect(container.querySelector('.home-talk-destination')?.textContent).toBe('To T3 Code');

    await renderBar('unavailable');
    expect(container.querySelector('.home-talk-destination')?.textContent).toBe('Clipboard only');
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
      'Home', 'Notetaker', 'Assistant', 'Insights',
    ]);
    expect(container.textContent).not.toContain('Customize');
    expect(container.querySelector('.home-sidebar-bottom')?.textContent).toContain('Dictation stays on this Mac.');
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
});
