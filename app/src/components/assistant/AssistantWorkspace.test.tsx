import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Markdown from 'react-markdown';
import { AssistantWorkspace, type AssistantWorkspaceProps } from './AssistantWorkspace';
import type { AssistantAction, AssistantActionStatus } from '../../lib/assistant';

// Count real Markdown renders while keeping parsing, sanitization and the
// resulting DOM in every workspace test.
vi.mock('react-markdown', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-markdown')>();
  const { createElement } = await import('react');
  return { ...actual, default: vi.fn((props: Parameters<typeof actual.default>[0]) => createElement(actual.default, props)) };
});

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
  beforeEach(() => { vi.mocked(Markdown).mockClear(); container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
  const button = (text: string) => Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.trim() === text)!;
  const render = async (p: AssistantWorkspaceProps) => act(async () => root.render(<AssistantWorkspace {...p} />));
  function scrollPane(top: number, height = 1000, viewportHeight = 400) {
    const pane = container.querySelector('[role="log"]') as HTMLDivElement;
    let scrollTop = top;
    let scrollHeight = height;
    let clientHeight = viewportHeight;
    const write = vi.fn((value: number) => { scrollTop = value; });
    Object.defineProperties(pane, {
      scrollTop: { configurable: true, get: () => scrollTop, set: write },
      scrollHeight: { configurable: true, get: () => scrollHeight },
      clientHeight: { configurable: true, get: () => clientHeight },
    });
    const scrollIntoView = vi.fn();
    for (const node of container.querySelectorAll<HTMLElement>('*')) node.scrollIntoView = scrollIntoView;
    return {
      pane, write, scrollIntoView,
      growTo: (height: number) => { scrollHeight = height; },
      resizeTo: (height: number) => { clientHeight = height; },
      scrollTo: async (top: number) => {
        scrollTop = top;
        await act(async () => pane.dispatchEvent(new Event('scroll')));
      },
    };
  }
  const reply = (text = 'An earlier reply', id = 'answer'): AssistantWorkspaceProps['messages'][number] => ({
    id, role: 'assistant', text, status: 'complete', actions: [],
  });
  it('skips unchanged Markdown while drafting and updates streamed text, actions and appended messages', async () => {
    const messages = Array.from({ length: 199 }, (_, index) => reply(`**Saved** reply ${index}`, `answer-${index}`));
    messages[0] = { ...messages[0], actions: [action()] };
    const p = props({ messages });
    await render(p);
    expect(Markdown).toHaveBeenCalledTimes(199);
    const input = container.querySelector('textarea')!;
    const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
    for (const draft of ['A', 'A new', 'A new draft']) {
      await act(async () => { set.call(input, draft); input.dispatchEvent(new Event('input', { bubbles: true })); });
    }
    expect(input.value).toBe('A new draft');
    expect(Markdown).toHaveBeenCalledTimes(199);

    // Full conversation hydration may replace every message object, even
    // when only admission state changed. Equal text still skips the parser.
    await render({ ...p, pending: true, messages: messages.map((message) => ({ ...message })) });
    expect(button('Confirm').disabled).toBe(true);
    expect(Markdown).toHaveBeenCalledTimes(199);

    const streamed = messages.map((message, index) => index === messages.length - 1
      ? { ...message, text: '**Streamed** response', status: 'running' as const }
      : { ...message });
    await render({ ...p, phase: 'running', messages: streamed });
    expect(container.querySelector('.assistant-message:last-of-type strong')?.textContent).toBe('Streamed');
    expect(container.querySelector('.assistant-message:last-of-type .assistant-turn-status')?.textContent).toContain('Pi is working');
    expect(Markdown).toHaveBeenCalledTimes(200);

    const finished = streamed.map((message, index) => ({ ...message, status: 'complete' as const,
      actions: index === 0 ? [action('uncertain')] : message.actions }));
    await render({ ...p, messages: finished });
    expect(container.querySelector('.assistant-turn-status')).toBeNull();
    expect(button('Confirm')).toBeUndefined();
    await act(async () => button('Refresh status').click());
    expect(p.onRefreshAction).toHaveBeenCalledWith(action().action_id);
    expect(Markdown).toHaveBeenCalledTimes(200);

    await render({ ...p, messages: [...finished, reply('**New** reply', 'new-answer')] });
    expect(container.querySelectorAll('.assistant-message')).toHaveLength(200);
    expect(container.querySelector('.assistant-message:last-of-type strong')?.textContent).toBe('New');
    expect(Markdown).toHaveBeenCalledTimes(201);
  });
  it('leaves scroll alone on unrelated rerenders, including equivalent message arrays', async () => {
    const p = props({ messages: [reply()] });
    await render(p);
    const scroll = scrollPane(600);
    container.scrollTop = 75;
    await render({ ...p, pending: true, messages: p.messages.map((message) => ({ ...message })) });
    expect(scroll.pane.scrollTop).toBe(600);
    expect(scroll.write).not.toHaveBeenCalled();
    expect(scroll.scrollIntoView).not.toHaveBeenCalled();
    expect(container.scrollTop).toBe(75);
  });
  it('follows appends and streamed growth locally while at the bottom', async () => {
    const p = props({ messages: [reply()] });
    await render(p);
    const scroll = scrollPane(600);
    await scroll.scrollTo(600);
    container.scrollTop = 75;
    scroll.growTo(1200);
    const appended = [...p.messages, { ...reply('First streamed words', 'next'), status: 'running' as const }];
    await render({ ...p, messages: appended, phase: 'running' });
    expect(scroll.pane.scrollTop).toBe(800);
    scroll.growTo(1450);
    await render({ ...p, messages: [p.messages[0], { ...appended[1], text: 'More streamed words' }], phase: 'running' });
    expect(scroll.pane.scrollTop).toBe(1050);
    expect(scroll.scrollIntoView).not.toHaveBeenCalled();
    expect(container.scrollTop).toBe(75);
  });
  it('preserves earlier-message reading through appends until the reader returns to the bottom', async () => {
    const p = props({ messages: [reply()] });
    await render(p);
    const scroll = scrollPane(600);
    await scroll.scrollTo(120);
    scroll.growTo(1250);
    const appended = [...p.messages, reply('A new reply', 'next')];
    await render({ ...p, messages: appended });
    expect(scroll.pane.scrollTop).toBe(120);
    expect(scroll.write).not.toHaveBeenCalled();
    expect(scroll.scrollIntoView).not.toHaveBeenCalled();
    await scroll.scrollTo(850);
    scroll.growTo(1500);
    await render({ ...p, messages: [...appended, reply('Another reply', 'last')] });
    expect(scroll.pane.scrollTop).toBe(1100);
  });
  it('opens a different conversation at its bottom after reading an earlier turn', async () => {
    const p = props({ messages: [reply()] });
    await render(p);
    const scroll = scrollPane(600);
    await scroll.scrollTo(120);
    scroll.growTo(1400);
    await render({ ...p, selectedId: 'another-thread', messages: [reply('Another conversation', 'other')] });
    expect(scroll.pane.scrollTop).toBe(1000);
    expect(scroll.scrollIntoView).not.toHaveBeenCalled();
    // Following is reset for the newly selected conversation as well.
    scroll.growTo(1600);
    await render({ ...p, selectedId: 'another-thread', messages: [reply('More content', 'other')] });
    expect(scroll.pane.scrollTop).toBe(1200);
  });
  it.each([
    { position: 'reading earlier messages', top: 120, following: false },
    { position: 'following the latest messages', top: 600, following: true },
  ])('preserves $position across a same-conversation disconnect/reconnect', async ({ top, following }) => {
    let height = 1000;
    const scrollHeight = vi.spyOn(Element.prototype, 'scrollHeight', 'get').mockImplementation(() => height);
    const clientHeight = vi.spyOn(Element.prototype, 'clientHeight', 'get').mockReturnValue(400);
    let onResize: ResizeObserverCallback | undefined;
    vi.stubGlobal('ResizeObserver', class {
      constructor(callback: ResizeObserverCallback) { onResize = callback; }
      observe() {}
      disconnect() {}
    });
    try {
      const p = props({ messages: [reply()] });
      await render(p);
      const originalPane = container.querySelector('[role="log"]') as HTMLDivElement;
      await act(async () => {
        originalPane.scrollTop = top;
        originalPane.dispatchEvent(new Event('scroll'));
      });
      await render({ ...p, connected: false });
      expect(container.querySelector('[role="log"]')).toBeNull();
      height = 1400;
      const messages = [...p.messages, reply('Arrived while disconnected', 'next')];
      await render({ ...p, messages });
      const restoredPane = container.querySelector('[role="log"]') as HTMLDivElement;
      expect(restoredPane).not.toBe(originalPane);
      expect(restoredPane.scrollTop).toBe(following ? 1000 : top);
      height = 1600;
      await act(async () => onResize!([], {} as ResizeObserver));
      expect(restoredPane.scrollTop).toBe(following ? 1200 : top);
      height = 1800;
      await render({ ...p, messages: [...messages, reply('Another new reply', 'last')] });
      expect(restoredPane.scrollTop).toBe(following ? 1400 : top);
    } finally {
      scrollHeight.mockRestore();
      clientHeight.mockRestore();
    }
  });
  it('follows content and pane resizing only while following, and disconnects its observer', async () => {
    let onResize: ResizeObserverCallback | undefined;
    const observe = vi.fn();
    const disconnect = vi.fn();
    vi.stubGlobal('ResizeObserver', class {
      constructor(callback: ResizeObserverCallback) { onResize = callback; }
      observe = observe;
      disconnect = disconnect;
    });
    const p = props({ messages: [reply()] });
    await render(p);
    const scroll = scrollPane(600);
    expect(observe).toHaveBeenCalledTimes(2);
    scroll.resizeTo(300);
    await act(async () => onResize!([], {} as ResizeObserver));
    expect(scroll.pane.scrollTop).toBe(700);
    await scroll.scrollTo(100);
    scroll.write.mockClear();
    scroll.growTo(1400);
    await act(async () => onResize!([], {} as ResizeObserver));
    expect(scroll.pane.scrollTop).toBe(100);
    expect(scroll.write).not.toHaveBeenCalled();
    await render({ ...p, connected: false });
    expect(disconnect).toHaveBeenCalledOnce();
  });
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
