import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MainHeader } from './MainHeader';
import { HistoryPanel } from './history/HistoryPanel';

describe('Sonic Canvas component details', () => {
  let container: HTMLDivElement;
  let root: Root;
  let writeText: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it.each([
    ['shift_l', '⇧ Shift'],
    ['alt_l', '⌥ Option'],
    ['ctrl_r', '⌃ Control'],
  ] as const)('shows the configured %s hotkey hint', async (triggerKey, label) => {
    await act(async () => {
      root.render(
        <MainHeader
          status="idle"
          initialized
          recordingDuration={0}
          recordingMode="hold_down"
          onRecord={vi.fn()}
          onStop={vi.fn()}
          onOpenSettings={vi.fn()}
          settingsOpen={false}
          triggerKey={triggerKey}
        />,
      );
    });

    expect(container.textContent).toContain(`Hold ${label} to dictate`);
  });

  it('shows a word-count badge on each history card', async () => {
    await act(async () => {
      root.render(
        <HistoryPanel
          entries={[{
            id: 'one',
            text: 'Semantic surfaces stay calm',
            timestamp: Date.UTC(2026, 6, 18, 12),
            duration: 3.1949375,
          }]}
          onClear={vi.fn()}
          onUpdateEntry={vi.fn()}
        />,
      );
    });

    expect(container.textContent).toContain('4 words');
    expect(container.textContent).toContain('3s');
    expect(container.textContent).not.toContain('3.1949375s');
  });

  it('preserves the idle and recording button actions', async () => {
    const onStart = vi.fn();
    const onStop = vi.fn();

    await act(async () => {
      root.render(
        <MainHeader
          status="idle"
          initialized
          recordingDuration={0}
          recordingMode="hold_down"
          onRecord={onStart}
          onStop={onStop}
          onOpenSettings={vi.fn()}
          settingsOpen={false}
          triggerKey="shift_l"
        />,
      );
    });
    await act(async () => container.querySelector('button')?.click());
    expect(onStart).toHaveBeenCalledOnce();

    await act(async () => {
      root.render(
        <MainHeader
          status="recording"
          initialized
          recordingDuration={12}
          recordingMode="hold_down"
          onRecord={onStart}
          onStop={onStop}
          onOpenSettings={vi.fn()}
          settingsOpen={false}
          triggerKey="shift_l"
        />,
      );
    });
    await act(async () => container.querySelector('button')?.click());
    expect(onStop).toHaveBeenCalledOnce();
  });

  it('keeps the recording control on the stable compact design contract', async () => {
    await act(async () => {
      root.render(
        <MainHeader
          status="recording"
          initialized
          recordingDuration={12}
          recordingMode="hold_down"
          onRecord={vi.fn()}
          onStop={vi.fn()}
          onOpenSettings={vi.fn()}
          settingsOpen={false}
          triggerKey="shift_l"
        />,
      );
    });

    const stop = container.querySelector('[data-testid="record-pill"]') as HTMLButtonElement;
    expect(stop.textContent).toContain('Stop 0:12');
    expect(stop.classList).toContain('ui-record-pill');
    expect(stop.classList).toContain('border');
    expect(stop.classList).toContain('border-error/50');
    expect(stop.classList).toContain('bg-error/10');
    expect(stop.classList).toContain('text-error');
    expect(stop.classList).not.toContain('bg-error');
    expect(stop.classList).not.toContain('text-on-primary');
  });

  it('keeps stable header geometry while state labels change', async () => {
    const renderHeader = async (status: 'idle' | 'recording' | 'processing') => {
      await act(async () => {
        root.render(
          <MainHeader
            status={status}
            initialized
            recordingDuration={12}
            recordingMode="hold_down"
            onRecord={vi.fn()}
            onStop={vi.fn()}
            onOpenSettings={vi.fn()}
            settingsOpen={false}
            triggerKey="shift_l"
          />,
        );
      });
      return {
        header: container.querySelector('header')!,
        statusChip: container.querySelector('[data-testid="main-status-chip"]')!,
        record: container.querySelector('[data-testid="record-pill"]')!,
      };
    };

    for (const status of ['idle', 'recording', 'processing'] as const) {
      const elements = await renderHeader(status);
      expect(elements.header.classList).toContain('ui-window-header');
      expect(elements.statusChip.classList).toContain('ui-status-chip');
      expect(elements.record.classList).toContain('ui-record-pill');
    }
  });

  it('preserves history copy and confirmed clear actions', async () => {
    const onClear = vi.fn();

    await act(async () => {
      root.render(
        <HistoryPanel
          entries={[{
            id: 'one',
            text: 'Keep every interaction working',
            timestamp: Date.UTC(2026, 6, 18, 12),
            duration: 3,
          }]}
          onClear={onClear}
          onUpdateEntry={vi.fn()}
        />,
      );
    });

    const copyButton = container.querySelector('[aria-label^="Copy transcription"]') as HTMLButtonElement;
    const counts = container.querySelector('[data-testid="transcript-counts"]')!;
    expect(counts.contains(copyButton)).toBe(false);
    expect(copyButton.closest('[data-testid="transcript-card"]')).not.toBeNull();
    await act(async () => copyButton.click());
    expect(writeText).toHaveBeenCalledWith('Keep every interaction working');

    // Clearing is a two-step confirm — the first click only arms it.
    const more = container.querySelector('[aria-label="More history actions"]') as HTMLButtonElement;
    await act(async () => more.click());
    const clearButton = Array.from(container.querySelectorAll('button')).find((candidate) => candidate.textContent === 'Clear history')!;
    await act(async () => clearButton.click());
    expect(onClear).not.toHaveBeenCalled();
    expect(clearButton.textContent).toBe('Clear all history?');
    await act(async () => clearButton.click());
    expect(onClear).toHaveBeenCalledOnce();
  });

  it('offers Correct and Teach only on the newest history entry', async () => {
    await act(async () => {
      root.render(<HistoryPanel entries={[
        { id: 'older', text: 'older transcript', timestamp: 1, duration: 1 },
        { id: 'newer', text: 'newest transcript', timestamp: 2, duration: 1 },
      ]} onClear={vi.fn()} onUpdateEntry={vi.fn()} />);
    });
    const actions = Array.from(container.querySelectorAll('button')).filter((candidate) => candidate.textContent === 'Correct & Teach');
    expect(actions).toHaveLength(1);
    await act(async () => actions[0].click());
    expect((container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement).value).toBe('newest transcript');
  });
});
