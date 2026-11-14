import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useBlockWindowParking, useWindowParked } from './windowParking';

const DELAY = 1_000;
let visibility: DocumentVisibilityState = 'visible';

function setVisibility(next: DocumentVisibilityState) {
  visibility = next;
  document.dispatchEvent(new Event('visibilitychange'));
}

let setBusy: (busy: boolean) => void = () => {};

function Probe() {
  const parked = useWindowParked(DELAY);
  return <div data-parked={String(parked)} />;
}

function Blocker() {
  const [busy, set] = useState(false);
  setBusy = set;
  useBlockWindowParking(busy);
  return null;
}

describe('window parking', () => {
  let container: HTMLDivElement;
  let root: Root;
  const parked = () => container.querySelector('[data-parked]')?.getAttribute('data-parked');

  beforeEach(async () => {
    vi.useFakeTimers();
    visibility = 'visible';
    vi.spyOn(document, 'visibilityState', 'get').mockImplementation(() => visibility);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<><Probe /><Blocker /></>));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('parks only after the window stays hidden for the delay', async () => {
    await act(async () => setVisibility('hidden'));
    await act(async () => vi.advanceTimersByTime(DELAY - 1));
    expect(parked()).toBe('false');
    await act(async () => vi.advanceTimersByTime(1));
    expect(parked()).toBe('true');
  });

  it('unparks as soon as the window is visible again', async () => {
    await act(async () => setVisibility('hidden'));
    await act(async () => vi.advanceTimersByTime(DELAY));
    await act(async () => setVisibility('visible'));
    expect(parked()).toBe('false');
  });

  it('does not park when the window is reopened before the delay', async () => {
    await act(async () => setVisibility('hidden'));
    await act(async () => vi.advanceTimersByTime(DELAY / 2));
    await act(async () => setVisibility('visible'));
    await act(async () => vi.advanceTimersByTime(DELAY * 2));
    expect(parked()).toBe('false');
  });

  it('waits for busy work to finish, then waits the full delay', async () => {
    await act(async () => setBusy(true));
    await act(async () => setVisibility('hidden'));
    await act(async () => vi.advanceTimersByTime(DELAY * 3));
    expect(parked()).toBe('false');
    await act(async () => setBusy(false));
    await act(async () => vi.advanceTimersByTime(DELAY - 1));
    expect(parked()).toBe('false');
    await act(async () => vi.advanceTimersByTime(1));
    expect(parked()).toBe('true');
  });

  it('a blocker that starts during the delay cancels the pending park', async () => {
    await act(async () => setVisibility('hidden'));
    await act(async () => vi.advanceTimersByTime(DELAY / 2));
    await act(async () => setBusy(true));
    await act(async () => vi.advanceTimersByTime(DELAY * 2));
    expect(parked()).toBe('false');
  });
});
