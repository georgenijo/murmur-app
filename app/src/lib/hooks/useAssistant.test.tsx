import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAssistant } from './useAssistant';
import type { AssistantConversation } from '../assistant';

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));
const id = '01234567-89ab-4cde-8fab-0123456789ab';
const requestId = '11234567-89ab-4cde-8fab-0123456789ab';
const options = { command: { provider: 'custom' as const, executable: '/test/pi', arguments: [], timeoutSeconds: 300, contextLevel: 'none' as const, retainQueryHistory: false }, deviceName: null, smartAuto: null };
const empty = (): AssistantConversation => ({ id, title: 'Existing chat', createdAtMs: 1, updatedAtMs: 2, messages: [], activePassId: null, liveState: null });

describe('Assistant conversation orchestration', () => {
  let root: Root; let container: HTMLDivElement; let state: ReturnType<typeof useAssistant>;
  let snapshot: AssistantConversation;
  const listeners = new Map<string, (e: { payload: unknown }) => void>();
  function Probe() { state = useAssistant(options); return null; }
  beforeEach(() => {
    snapshot = empty(); listeners.clear(); mocks.invoke.mockReset(); mocks.listen.mockReset();
    mocks.listen.mockImplementation(async (name, callback) => { listeners.set(name, callback); return () => listeners.delete(name); });
    mocks.invoke.mockImplementation(async (name) => {
      if (name === 'get_assistant_connection') return { enabled: true };
      if (name === 'list_assistant_conversations') return [{ id, title: 'Existing chat', createdAtMs: 1, updatedAtMs: 2, messageCount: snapshot.messages.length }];
      if (name === 'get_assistant_conversation') return { ...snapshot };
      if (name === 'cancel_query') { snapshot = { ...snapshot, activePassId: null, liveState: null }; return; }
      throw new Error('unexpected test command');
    });
    container = document.createElement('div'); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  const mount = () => act(async () => root.render(<Probe />));
  it('hydrates native history without enabling consent or resending any message', async () => {
    await mount();
    expect(state.connected).toBe(true); expect(state.conversation?.id).toBe(id);
    expect(mocks.invoke.mock.calls.some(([name]) => name === 'connect_assistant' || name === 'send_assistant_message')).toBe(false);
  });
  it('does not downgrade Listening when microphone event arrives before admission response', async () => {
    await mount();
    const original = mocks.invoke.getMockImplementation()!;
    let resolveAdmission!: (v: unknown) => void;
    mocks.invoke.mockImplementation((name, args) => name === 'start_assistant_voice' ? new Promise((resolve) => { resolveAdmission = resolve; }) : original(name, args));
    let started!: Promise<void>;
    await act(async () => { started = state.voice(); });
    snapshot = { ...snapshot, activePassId: 5, liveState: 'listening' };
    await act(async () => { listeners.get('assistant-state-changed')!({ payload: { conversationId: id, queryPassId: 5, state: 'listening', errorCode: null } }); });
    await act(async () => { resolveAdmission({ conversationId: id, queryPassId: 5, requestId }); await started; });
    expect(state.phase).toBe('listening');
  });
  it('blocks dispatch if event registration fails instead of starting an unobservable microphone', async () => {
    mocks.listen.mockRejectedValue(new Error('unavailable'));
    await mount();
    expect(state.pending).toBe(true);
    await act(async () => { await state.voice(); });
    expect(mocks.invoke.mock.calls.some(([name]) => name === 'start_assistant_voice')).toBe(false);
  });
  it('can stop while finish-speaking IPC is still pending', async () => {
    snapshot = { ...snapshot, activePassId: 5, liveState: 'listening' };
    await mount();
    const original = mocks.invoke.getMockImplementation()!;
    let resolveFinish!: () => void;
    mocks.invoke.mockImplementation((name, args) => name === 'finish_query_capture' ? new Promise<void>((resolve) => { resolveFinish = resolve; }) : original(name, args));
    let finishing!: Promise<void>;
    await act(async () => { finishing = state.finishVoice(); });
    await act(async () => { await state.stop(); });
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_query', { queryPassId: 5 });
    await act(async () => { resolveFinish(); await finishing; });
  });
  it('cancels the admitted owner when Escape precedes the admission response', async () => {
    await mount();
    const original = mocks.invoke.getMockImplementation()!;
    let resolveAdmission!: (v: unknown) => void;
    mocks.invoke.mockImplementation((name, args) => name === 'start_assistant_voice' ? new Promise((resolve) => { resolveAdmission = resolve; }) : original(name, args));
    let started!: Promise<void>;
    await act(async () => { started = state.voice(); });
    await act(async () => { await state.stop(); });
    await act(async () => { resolveAdmission({ conversationId: id, queryPassId: 9, requestId }); await started; });
    expect(mocks.invoke).toHaveBeenCalledWith('cancel_query', { queryPassId: 9 });
    expect(state.phase).toBe('idle');
  });
});
