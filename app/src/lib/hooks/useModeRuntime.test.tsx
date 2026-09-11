import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  listener: null as null | ((event: { payload: unknown }) => void),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

import { useModeRuntime, type ModeRuntimeStatus } from './useModeRuntime';

describe('useModeRuntime', () => {
  let root: Root;
  let container: HTMLDivElement;
  let api: ReturnType<typeof useModeRuntime> | null = null;

  function Harness() {
    api = useModeRuntime();
    return <span>{api.status.name}:{api.status.source}</span>;
  }

  beforeEach(async () => {
    mocks.listener = null;
    mocks.invoke.mockReset();
    mocks.listen.mockReset();
    mocks.invoke.mockResolvedValue({ id: 'builtin.notes', name: 'Notes', source: 'manual' });
    mocks.listen.mockImplementation(async (_name, listener) => {
      mocks.listener = listener;
      return () => { mocks.listener = null; };
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(<Harness />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('loads, follows native app-binding transitions, and cycles through Rust', async () => {
    expect(container.textContent).toBe('Notes:manual');
    await act(async () => mocks.listener?.({
      payload: { id: 'builtin.email', name: 'Email', source: 'app_binding' },
    }));
    expect(container.textContent).toBe('Email:app_binding');
    await act(async () => mocks.listener?.({
      payload: { id: 'builtin.technical', name: 'Technical', source: 'site_binding' },
    }));
    expect(container.textContent).toBe('Technical:site_binding');
    mocks.invoke.mockResolvedValueOnce({ id: 'builtin.verbatim', name: 'Verbatim', source: 'temporary' });
    await act(async () => { await api?.cycle(); });
    expect(mocks.invoke).toHaveBeenLastCalledWith('cycle_mode');
    expect(container.textContent).toBe('Verbatim:temporary');
  });

  it('rejects malformed native payloads', async () => {
    await act(async () => mocks.listener?.({ payload: { id: '', name: 'Private', source: 'manual' } }));
    expect(container.textContent).toBe('Notes:manual');
  });
});

it('ignores a stale response or event after pending intent has been consumed', async () => {
  let listener: (event: { payload: unknown }) => void = () => {};
  let resolveInitial: (status: ModeRuntimeStatus) => void = () => {};
  const initial = new Promise<ModeRuntimeStatus>((resolve) => { resolveInitial = resolve; });
  const unlisten = vi.fn();
  mocks.invoke.mockReturnValue(initial);
  mocks.listen.mockImplementation(async (_name, callback) => { listener = callback; return unlisten; });
  const container = document.createElement('div');
  const root = createRoot(container);
  function Harness() {
    const { status } = useModeRuntime();
    return <span>{status.pending?.name ?? 'none'}</span>;
  }
  const status = { id: 'builtin.email', name: 'Email', source: 'app_binding' as const, available: [] };
  await act(async () => root.render(<Harness />));
  await act(async () => listener({ payload: { ...status, pendingRevision: 1, pending: { id: 'builtin.verbatim', name: 'Verbatim' } } }));
  expect(container.textContent).toBe('Verbatim');
  await act(async () => resolveInitial({ ...status, pendingRevision: 0, pending: null }));
  expect(container.textContent).toBe('Verbatim');
  await act(async () => listener({ payload: { ...status, pendingRevision: 2, pending: null } }));
  await act(async () => listener({ payload: { ...status, pendingRevision: 1, pending: { id: 'builtin.verbatim', name: 'Verbatim' } } }));
  expect(container.textContent).toBe('none');
  await act(async () => root.unmount());
  expect(unlisten).toHaveBeenCalledOnce();
});
