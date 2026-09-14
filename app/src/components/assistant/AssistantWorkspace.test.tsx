import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AssistantWorkspace, type AssistantWorkspaceProps } from './AssistantWorkspace';
import type { AssistantAction, AssistantActionStatus } from '../../lib/assistant';

const action = (status: AssistantActionStatus = 'proposed'): AssistantAction => ({
  schema_version: 1,
  action_id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
  idempotency_key: '0'.repeat(64),
  kind: 'lights.set',
  actor_id: 'george',
  connection_id: '11111111-1111-4111-8111-111111111111',
  conversation_id: '22222222-2222-4222-8222-222222222222',
  request_id: '33333333-3333-4333-8333-333333333333',
  targets: [{ entity_id: 'light.govee_bulb_149d', name: 'Living room bulb' }],
  parameters: { power: 'on', brightness_pct: 30, rgb_color: [0, 0, 255] },
  created_at: '2026-09-14T04:00:00Z',
  expires_at: '2026-09-14T04:05:00Z',
  parameter_digest: '1'.repeat(64),
  status,
  receipt: null,
  verification: null,
});

function props(overrides: Partial<AssistantWorkspaceProps> = {}): AssistantWorkspaceProps {
  return {
    connected: true, configured: true, loading: false, pending: false,
    conversations: [{ id: 'thread', title: 'Bedroom lights', updatedAtMs: 1700000000000 }], selectedId: 'thread', messages: [], confirmationOutcomeUnknownActionIds: [], phase: 'idle', error: null, voiceAvailable: true,
    onConnect: vi.fn(async () => {}), onDisconnect: vi.fn(async () => {}), onNew: vi.fn(async () => {}), onSelect: vi.fn(async () => {}), onDelete: vi.fn(async () => {}),
    onSend: vi.fn(async () => true), onVoice: vi.fn(async () => {}), onFinishVoice: vi.fn(async () => {}), onStop: vi.fn(async () => {}),
    onConfirmAction: vi.fn(async () => {}), onCancelAction: vi.fn(async () => {}), onRefreshAction: vi.fn(async () => {}), onSettings: vi.fn(), ...overrides,
  };
}
describe('Assistant workspace', () => {
  let container: HTMLDivElement; let root: Root;
  beforeEach(() => { container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  const button = (text: string) => Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.trim() === text)!;
  const render = async (p: AssistantWorkspaceProps) => act(async () => root.render(<AssistantWorkspace {...p} />));
  it('obtains explicit persistence consent and never connects merely by rendering', async () => {
    const p = props({ connected: false }); await render(p);
    expect(container.textContent).toContain('saved on this Mac');
    expect(container.textContent).toContain('Ubuntu');
    expect(p.onConnect).not.toHaveBeenCalled();
    await act(async () => button('Connect Pi Assistant').click());
    expect(p.onConnect).toHaveBeenCalledOnce();
  });
  it('does not offer connection before a compatible Custom executable is configured', async () => {
    await render(props({ connected: false, configured: false }));
    expect(button('Connect Pi Assistant').disabled).toBe(true);
  });
  it('sends typed text, preserves newlines, and clears only accepted sends', async () => {
    const p = props(); await render(p);
    const input = container.querySelector('textarea')!;
    const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
    await act(async () => { set.call(input, 'First line\nSecond line'); input.dispatchEvent(new Event('input', { bubbles: true })); });
    await act(async () => container.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
    expect(p.onSend).toHaveBeenCalledWith('First line\nSecond line');
    expect(input.value).toBe('');
  });
  it('keeps voice Finish and Stop separate and Escape cancels the active reply', async () => {
    const p = props({ phase: 'listening' }); await render(p);
    await act(async () => button('Finish speaking').click()); expect(p.onFinishVoice).toHaveBeenCalledOnce();
    await act(async () => button('Stop').click()); expect(p.onStop).toHaveBeenCalledOnce();
    await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(p.onStop).toHaveBeenCalledTimes(2);
    expect((container.querySelector('[aria-label="New chat"]') as HTMLButtonElement).disabled).toBe(true);
  });
  it('requires local-only deletion confirmation and never claims remote erasure', async () => {
    const p = props(); await render(p);
    const options = container.querySelector('.assistant-chat-options') as HTMLDetailsElement;
    expect(options.open).toBe(false);
    await act(async () => options.querySelector('summary')!.click());
    await act(async () => button('Delete chat…').click());
    expect(options.open).toBe(false);
    expect(p.onDelete).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(button('Keep chat'));
    expect(container.textContent).toContain('Pi’s saved session on Ubuntu is not deleted');
    await act(async () => button('Delete from this Mac').click());
    expect(p.onDelete).toHaveBeenCalledWith('thread');
  });
  it('keeps connection details tucked away without changing consent or tool access', async () => {
    const p = props({ phase: 'running' }); await render(p);
    const details = container.querySelector('.assistant-connection') as HTMLDetailsElement;
    expect(details.open).toBe(false);
    await act(async () => details.querySelector('summary')!.click());
    expect(details.open).toBe(true);
    expect(details.textContent).toContain('Light changes always require confirmation here');
    expect(p.onDisconnect).not.toHaveBeenCalled();
    await act(async () => details.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(details.open).toBe(false);
    expect(p.onStop).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(details.querySelector('summary'));
  });
  it.each(['listening', 'running'] as const)('dismisses each menu without stopping a %s turn', async (phase) => {
    const p = props({ phase }); await render(p);
    for (const details of container.querySelectorAll('details')) {
      await act(async () => details.querySelector('summary')!.click());
      await act(async () => details.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
      expect(details.open).toBe(false);
      expect(document.activeElement).toBe(details.querySelector('summary'));
      expect(p.onStop).not.toHaveBeenCalled();
    }
    await act(async () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(p.onStop).toHaveBeenCalledOnce();
  });
  it('returns focus to Chat options after keeping a conversation', async () => {
    const p = props(); await render(p);
    await act(async () => container.querySelector('.assistant-chat-options summary')!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    await act(async () => button('Delete chat…').click());
    expect(document.activeElement).toBe(button('Keep chat'));
    await act(async () => button('Keep chat').click());
    expect(document.activeElement).toBe(container.querySelector('.assistant-chat-options summary'));
    expect(p.onDelete).not.toHaveBeenCalled();
  });
  it('renders saved turns with incomplete status and never loads model-supplied images or links', async () => {
    await render(props({ messages: [{ id: 'answer', role: 'assistant', text: '![track](https://example.invalid/pixel) [click](https://example.invalid/) **On**', status: 'cancelled', actions: [] }] }));
    expect(container.querySelector('img')).toBeNull(); expect(container.querySelector('a')).toBeNull();
    expect(container.textContent).toContain('Stopped. This response may be incomplete.');
    expect(container.querySelector('strong')?.textContent).toBe('On');
  });
  it('dictates into the composer, preserves typed prefix and sends only on explicit submit', async () => {
    const p = props(); await render(p);
    const input = container.querySelector('textarea')!;
    const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
    await act(async () => { set.call(input, 'My typed note'); input.dispatchEvent(new Event('input', { bubbles: true })); });
    await act(async () => (container.querySelector('[aria-label="Dictate message"]') as HTMLButtonElement).click());
    await render({ ...p, phase: 'listening', dictation: { passId: 5, text: 'Live speech', status: 'partial' } });
    expect(input.value).toBe('My typed note\nLive speech');
    expect(input.readOnly).toBe(true);
    expect(p.onSend).not.toHaveBeenCalled();
    await act(async () => button('Finish speaking').click());
    expect(p.onFinishVoice).toHaveBeenCalledOnce();
    await render({ ...p, dictation: { passId: 5, text: 'Final speech', status: 'final' } });
    expect(input.readOnly).toBe(false);
    expect(input.value).toBe('My typed note\nFinal speech');
    expect(p.onSend).not.toHaveBeenCalled();
    await act(async () => container.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true })));
    expect(p.onSend).toHaveBeenCalledWith('My typed note\nFinal speech');
  });
  it('restores the typed draft when dictation is cancelled', async () => {
    const p = props(); await render(p);
    const input = container.querySelector('textarea')!;
    const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
    await act(async () => { set.call(input, 'Keep me'); input.dispatchEvent(new Event('input', { bubbles: true })); });
    await act(async () => (container.querySelector('[aria-label="Dictate message"]') as HTMLButtonElement).click());
    await render({ ...p, phase: 'listening', dictation: { passId: 5, text: 'Discard me', status: 'partial' } });
    await render({ ...p, dictation: { passId: 5, text: '', status: 'cancelled' } });
    expect(input.value).toBe('Keep me');
    expect(p.onSend).not.toHaveBeenCalled();
  });
  it('shows exact proposed action details and sends only the action ID to Confirm or Cancel', async () => {
    const p = props({ messages: [{ id: 'answer', role: 'assistant', text: '', status: 'complete', actions: [action()] }] });
    await render(p);
    expect(container.textContent).toContain('Pending approval');
    expect(container.textContent).toContain('Blue (RGB 0, 0, 255)');
    expect(container.textContent).toContain('light.govee_bulb_149d');
    await act(async () => button('Confirm').click());
    await act(async () => button('Cancel').click());
    expect(p.onConfirmAction).toHaveBeenCalledWith('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa');
    expect(p.onCancelAction).toHaveBeenCalledWith('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa');
  });
  it('disables duplicate action clicks while busy and offers status-only recovery', async () => {
    const p = props({ pending: true, messages: [{ id: 'answer', role: 'assistant', text: '', status: 'complete', actions: [action()] }] });
    await render(p);
    expect(button('Confirm').disabled).toBe(true);
    expect(button('Cancel').disabled).toBe(true);
    await act(async () => button('Confirm').click());
    expect(p.onConfirmAction).not.toHaveBeenCalled();

    const uncertain = props({ messages: [{ id: 'answer', role: 'assistant', text: '', status: 'complete', actions: [action('uncertain')] }] });
    await render(uncertain);
    expect(container.textContent).toContain('Uncertain');
    await act(async () => button('Refresh status').click());
    expect(uncertain.onRefreshAction).toHaveBeenCalledWith('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa');
    expect(uncertain.onConfirmAction).not.toHaveBeenCalled();
  });
  it('keeps an interrupted confirmation visibly unknown and permits only status refresh', async () => {
    const p = props({
      confirmationOutcomeUnknownActionIds: ['aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'],
      messages: [{ id: 'answer', role: 'assistant', text: '', status: 'complete', actions: [action()] }],
    });
    await render(p);
    expect(container.textContent).toContain('Confirmation outcome unknown');
    expect(container.textContent).toContain('Last known server state: Pending approval');
    expect(button('Confirm')).toBeUndefined();
    expect(button('Cancel')).toBeUndefined();
    await act(async () => button('Refresh status').click());
    expect(p.onRefreshAction).toHaveBeenCalledWith('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa');
  });
});
