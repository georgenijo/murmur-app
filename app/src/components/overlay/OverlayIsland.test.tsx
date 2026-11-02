import geometryFixture from './overlay-geometry.fixture.json';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { OverlayGeometry } from '../../lib/overlayGeometry';
import { OverlayIsland } from './OverlayIsland';

const pillExpanded = geometryFixture.pillExpanded as OverlayGeometry;
const pillRecording = geometryFixture.pillRecording as OverlayGeometry;

describe('OverlayIsland floating pill', () => {
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

  const render = (geometry: OverlayGeometry, expanded: boolean) => act(async () => {
    root.render(
      <OverlayIsland geometry={geometry} expanded={expanded} active>
        <div data-testid="row" />
      </OverlayIsland>,
    );
  });
  const island = () => container.querySelector<HTMLDivElement>('.overlay-island')!;
  const frame = () => container.querySelector<HTMLDivElement>('.overlay-pill-frame')!;

  // Closing: the native window keeps its expanded size until the transition
  // settles, so the capsule must contract both dimensions inside it instead of
  // collapsing its height at the expanded width.
  it('contracts width with height while the window is still expanded', async () => {
    await render(pillExpanded, true);
    expect(island().style.width).toBe(`${pillExpanded.windowW}px`);
    await render(pillExpanded, false);
    expect(island().style.width).toBe(`${pillExpanded.island.pillW}px`);
    expect(island().style.height).toBe(`${pillExpanded.collapsedH}px`);
  });

  it('keeps children in a window-width frame so they never reflow', async () => {
    await render(pillExpanded, false);
    expect(frame().style.width).toBe(`${pillExpanded.windowW - 2}px`);
    await render(pillRecording, false);
    expect(island().style.width).toBe(`${pillRecording.island.pillW}px`);
    expect(frame().style.width).toBe(`${pillRecording.windowW - 2}px`);
  });

  it('leaves the notch island at the window width with a layout-neutral frame', async () => {
    const notched = geometryFixture.notched as OverlayGeometry;
    await render(notched, false);
    expect(island().style.width).toBe(`${notched.windowW}px`);
    expect(frame().style.display).toBe('contents');
  });

  it('keeps its rows mounted across a placement switch', async () => {
    await render(geometryFixture.recordingNotched as OverlayGeometry, false);
    const row = container.querySelector('[data-testid="row"]');
    await render(pillRecording, false);
    expect(container.querySelector('[data-testid="row"]')).toBe(row);
  });
});
