import { act, type MouseEvent } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ info: vi.fn() }));
vi.mock('../log', () => ({ flog: { info: mocks.info } }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => false) }));
vi.mock('../targetedEvents', () => ({ listen: vi.fn(async () => () => {}) }));
import { useRecordingControls } from './useRecordingControls';

it('logs only bounded class names for SVG mousedown targets', async () => {
  const container = document.createElement('div');
  const root = createRoot(container);
  let controls!: ReturnType<typeof useRecordingControls>;
  function Harness() {
    controls = useRecordingControls({
      status: 'recording', statusRef: { current: 'recording' },
      disabledRef: { current: false }, expandedRef: { current: false },
    });
    return null;
  }
  try {
    await act(async () => root.render(<Harness />));
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    svg.setAttribute('class', 'a'.repeat(80));
    svg.textContent = 'synthetic text excluded from diagnostics';
    expect(() => controls.handleMouseDown({ target: svg, button: 0, clientX: 1, clientY: 2 } as unknown as MouseEvent)).not.toThrow();
    expect(mocks.info).toHaveBeenCalledWith('overlay', 'MOUSEDOWN', {
      button: 0, x: 1, y: 2, target: 'path', className: 'a'.repeat(50), locked: false, status: 'recording',
    });
  } finally {
    await act(async () => root.unmount());
  }
});
