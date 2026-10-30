import geometryFixture from '../../components/overlay/overlay-geometry.fixture.json';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { OverlayGeometry } from '../overlayGeometry';
import { useApplySavedOverlayOffset } from './useApplySavedOverlayOffset';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

const geometry: OverlayGeometry = {
  windowW: 280,
  collapsedH: 30,
  expandedH: 94,
  pillIdleW: 280,
  pillActiveW: 280,
  pillMarginIdle: 0,
  pillMarginActive: 0,
  dropdownH: 64,
  wingW: 36,
  floating: true,
  island: geometryFixture.external.island,
};

function Harness({ value, savedOffset }: {
  value: OverlayGeometry | null;
  savedOffset: number;
}) {
  useApplySavedOverlayOffset(value, savedOffset);
  return null;
}

describe('useApplySavedOverlayOffset', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
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

  it('does not replay the saved offset over a live draft on geometry refresh', async () => {
    await act(async () => root.render(<Harness value={geometry} savedOffset={4} />));
    expect(mocks.invoke).toHaveBeenCalledWith('set_overlay_vertical_offset', { offset: 4 });

    // Settings calibration now owns an unsaved native draft. A new geometry
    // object represents the display/placement refresh that used to replay 4.
    mocks.invoke.mockClear();
    await act(async () => root.render(
      <Harness value={{ ...geometry, collapsedH: 24, expandedH: 88 }} savedOffset={4} />,
    ));
    expect(mocks.invoke).not.toHaveBeenCalled();

    await act(async () => root.render(
      <Harness value={{ ...geometry, collapsedH: 24, expandedH: 88 }} savedOffset={5} />,
    ));
    expect(mocks.invoke).toHaveBeenCalledWith('set_overlay_vertical_offset', { offset: 5 });
  });
});
