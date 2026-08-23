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

  it('routes main and customization destinations without a duplicate Settings entry', async () => {
    const onNavigate = vi.fn();
    const onOpenSettings = vi.fn();
    await act(async () => root.render(
      <HomeSidebar active="home" onNavigate={onNavigate} onOpenSettings={onOpenSettings} />,
    ));

    const button = (label: string) => Array.from(container.querySelectorAll('button'))
      .find((candidate) => candidate.getAttribute('aria-label') === label) as HTMLButtonElement;
    await act(async () => button('Insights').click());
    await act(async () => button('Text & Vocabulary').click());
    await act(async () => button('Voice Commands').click());
    await act(async () => button('Styles').click());
    await act(async () => button('Transforms').click());

    expect(onNavigate).toHaveBeenCalledWith('insights');
    expect(onOpenSettings).toHaveBeenNthCalledWith(1, { page: 'text' });
    expect(onOpenSettings).toHaveBeenNthCalledWith(2, { page: 'text', editorTab: 'commands' });
    expect(onOpenSettings).toHaveBeenNthCalledWith(3, { page: 'delivery', target: 'app-overrides' });
    expect(onOpenSettings).toHaveBeenNthCalledWith(4, { page: 'ai-transform' });
    expect(button('Settings')).toBeUndefined();
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
});
