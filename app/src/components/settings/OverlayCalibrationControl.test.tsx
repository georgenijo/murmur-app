import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { OverlayCalibrationControl } from './OverlayCalibrationControl';

const mocks = vi.hoisted(() => ({
  emit: vi.fn(),
  invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ emit: mocks.emit }));

function click(container: HTMLElement, label: string): void {
  const button = Array.from(container.querySelectorAll('button')).find(
    (candidate) => candidate.textContent?.trim() === label
      || candidate.getAttribute('aria-label') === label,
  );
  if (!button) throw new Error(`Missing button: ${label}`);
  button.click();
}

describe('OverlayCalibrationControl', () => {
  let container: HTMLDivElement;
  let root: Root;
  let committed: number[];

  function Harness({ initialOffset }: { initialOffset: number }) {
    const [offset, setOffset] = useState(initialOffset);
    return (
      <OverlayCalibrationControl
        offset={offset}
        onCommit={(next) => {
          committed.push(next);
          setOffset(next);
        }}
      />
    );
  }

  beforeEach(async () => {
    committed = [];
    mocks.emit.mockReset();
    mocks.emit.mockResolvedValue(undefined);
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(undefined);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('previews native movement and restores the original position on cancel', async () => {
    await act(async () => root.render(<Harness initialOffset={4} />));

    await act(async () => {
      click(container, 'Calibrate');
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.invoke.mock.calls.slice(0, 2)).toEqual([
      ['show_overlay'],
      ['set_overlay_vertical_offset', { offset: 4 }],
    ]);
    expect(mocks.emit).toHaveBeenCalledWith('overlay-calibration-changed', { active: true });

    await act(async () => {
      click(container, 'Move overlay up one point');
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenLastCalledWith('set_overlay_vertical_offset', { offset: 3 });

    await act(async () => {
      click(container, 'Cancel');
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenLastCalledWith('set_overlay_vertical_offset', { offset: 4 });
    expect(mocks.emit).toHaveBeenLastCalledWith('overlay-calibration-changed', { active: false });
    expect(committed).toEqual([]);
    expect(container.textContent).toContain('+4 pt');
  });

  it('commits only the position explicitly saved by the user', async () => {
    await act(async () => root.render(<Harness initialOffset={0} />));
    await act(async () => {
      click(container, 'Calibrate');
      await Promise.resolve();
      await Promise.resolve();
    });
    await act(async () => {
      click(container, 'Move overlay down one point');
      await Promise.resolve();
    });
    await act(async () => click(container, 'Save position'));

    expect(committed).toEqual([1]);
    expect(container.textContent).toContain('+1 pt');
    expect(mocks.emit).toHaveBeenLastCalledWith('overlay-calibration-changed', { active: false });
  });

  it('resets a saved offset immediately from the inactive state', async () => {
    await act(async () => root.render(<Harness initialOffset={-6} />));
    await act(async () => {
      click(container, 'Reset');
      await Promise.resolve();
    });

    expect(mocks.invoke).toHaveBeenCalledWith('set_overlay_vertical_offset', { offset: 0 });
    expect(committed).toEqual([0]);
    expect(container.textContent).toContain('Default position');
  });

  it('keeps calibration inactive when the native overlay cannot be opened', async () => {
    mocks.invoke.mockRejectedValueOnce(new Error('window unavailable'));
    await act(async () => root.render(<Harness initialOffset={0} />));
    await act(async () => {
      click(container, 'Calibrate');
      await Promise.resolve();
    });

    expect(container.querySelector('[role="alert"]')?.textContent).toContain('could not start');
    expect(container.textContent).not.toContain('Save position');
    expect(mocks.emit).not.toHaveBeenCalled();
  });
});
