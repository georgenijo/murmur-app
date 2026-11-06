import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { useLocalStats } from './useLocalStats';
import { COMPLETION_CHANGED } from '../completionData';
const mocks = vi.hoisted(() => ({ listen: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
it('multiple mounted readers only refresh presentation and never subscribe to completion producers', async () => {
  const root = createRoot(document.createElement('div'));
  let versions: number[] = [];
  function Harness() { versions = [useLocalStats(), useLocalStats()]; return null; }
  await act(async () => root.render(<Harness />));
  await act(async () => window.dispatchEvent(new Event(COMPLETION_CHANGED)));
  expect(versions).toEqual([1, 1]);
  expect(mocks.listen).not.toHaveBeenCalled();
  expect(localStorage.getItem('dictation-stats')).toBeNull();
  await act(async () => root.unmount());
});
