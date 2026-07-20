import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { RecordingControls } from './RecordingControls';
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
        <RecordingControls
          status="idle"
          initialized
          onStart={vi.fn()}
          onStop={vi.fn()}
          triggerKey={triggerKey}
        />,
      );
    });

    expect(container.textContent).toContain(`Press ${label} to begin`);
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
          onClearHistory={vi.fn()}
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
        <RecordingControls
          status="idle"
          initialized
          onStart={onStart}
          onStop={onStop}
          triggerKey="shift_l"
        />,
      );
    });
    await act(async () => container.querySelector('button')?.click());
    expect(onStart).toHaveBeenCalledOnce();

    await act(async () => {
      root.render(
        <RecordingControls
          status="recording"
          initialized
          onStart={onStart}
          onStop={onStop}
          triggerKey="shift_l"
        />,
      );
    });
    await act(async () => container.querySelector('button')?.click());
    expect(onStop).toHaveBeenCalledOnce();
  });

  it('preserves history copy and confirmed clear actions', async () => {
    const onClearHistory = vi.fn();
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    localStorage.setItem('dictation-history', 'saved');

    await act(async () => {
      root.render(
        <HistoryPanel
          entries={[{
            id: 'one',
            text: 'Keep every interaction working',
            timestamp: Date.UTC(2026, 6, 18, 12),
            duration: 3,
          }]}
          onClearHistory={onClearHistory}
          onUpdateEntry={vi.fn()}
        />,
      );
    });

    const copyButton = container.querySelector('[aria-label^="Copy transcription"]') as HTMLButtonElement;
    const clearButton = Array.from(container.querySelectorAll('button')).find((candidate) => candidate.textContent === 'Clear History')!;
    await act(async () => copyButton.click());
    expect(writeText).toHaveBeenCalledWith('Keep every interaction working');

    await act(async () => clearButton.click());
    expect(onClearHistory).toHaveBeenCalledOnce();
    expect(localStorage.getItem('dictation-history')).toBeNull();
  });

  it('offers Correct and Teach only on the newest history entry', async () => {
    await act(async () => {
      root.render(<HistoryPanel entries={[
        { id: 'older', text: 'older transcript', timestamp: 1, duration: 1 },
        { id: 'newer', text: 'newest transcript', timestamp: 2, duration: 1 },
      ]} onClearHistory={vi.fn()} onUpdateEntry={vi.fn()} />);
    });
    const actions = Array.from(container.querySelectorAll('button')).filter((candidate) => candidate.textContent === 'Correct and Teach');
    expect(actions).toHaveLength(1);
    await act(async () => actions[0].click());
    expect((container.querySelector('[aria-label="Corrected transcript"]') as HTMLTextAreaElement).value).toBe('newest transcript');
  });
});
