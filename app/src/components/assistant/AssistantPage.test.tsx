import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { AssistantPage } from './AssistantPage';
import type { AssistantWorkspaceProps } from './AssistantWorkspace';

const mocks = vi.hoisted(() => ({ useAssistant: vi.fn(), workspace: vi.fn((_props: AssistantWorkspaceProps) => null) }));
vi.mock('../../lib/hooks/useAssistant', () => ({ useAssistant: mocks.useAssistant }));
vi.mock('./AssistantWorkspace', () => ({ AssistantWorkspace: mocks.workspace }));

it('keeps messages stable through unrelated updates and publishes changed message content', async () => {
  const container = document.createElement('div');
  const root = createRoot(container);
  const sourceMessages = [{ id: 'answer', role: 'assistant', content: 'Original reply', actions: [], status: 'ready' }];
  const state = { connected: true, conversation: { id: 'thread', messages: sourceMessages }, phase: 'idle' };
  const props = {
    command: { provider: 'custom' as const, executable: '/fixture/assistant', arguments: [], timeoutSeconds: 60, contextLevel: 'none' as const, retainQueryHistory: false },
    deviceName: null, smartAuto: null, voiceAvailable: true, requestedId: null, onSettings: vi.fn(),
  };
  const messages = () => mocks.workspace.mock.calls[mocks.workspace.mock.calls.length - 1][0].messages;
  try {
    mocks.useAssistant.mockReturnValue(state);
    await act(async () => root.render(<AssistantPage {...props} />));
    const first = messages();
    expect(first[0]).toMatchObject({ text: 'Original reply', status: 'complete' });
    mocks.useAssistant.mockReturnValue({ ...state, phase: 'running', conversation: { ...state.conversation } });
    await act(async () => root.render(<AssistantPage {...props} voiceAvailable={false} />));
    expect(messages()).toBe(first);
    mocks.useAssistant.mockReturnValue({ ...state, conversation: { ...state.conversation, messages: [{ ...sourceMessages[0], content: 'Updated reply' }] } });
    await act(async () => root.render(<AssistantPage {...props} />));
    expect(messages()).not.toBe(first);
    expect(messages()[0].text).toBe('Updated reply');
  } finally {
    await act(async () => root.unmount());
  }
});
