import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AssistantWorkspace, type AssistantWorkspaceProps } from './AssistantWorkspace';

function props(overrides: Partial<AssistantWorkspaceProps> = {}): AssistantWorkspaceProps {
  return {
    connected: true, configured: true, loading: false, pending: false,
    conversations: [{ id: 'thread', title: 'Bedroom lights', updatedAtMs: 1700000000000 }], selectedId: 'thread', messages: [], phase: 'idle', error: null, voiceAvailable: true,
    onConnect: vi.fn(async () => {}), onDisconnect: vi.fn(async () => {}), onNew: vi.fn(async () => {}), onSelect: vi.fn(async () => {}), onDelete: vi.fn(async () => {}),
    onSend: vi.fn(async () => true), onVoice: vi.fn(async () => {}), onFinishVoice: vi.fn(async () => {}), onStop: vi.fn(async () => {}), onSettings: vi.fn(), ...overrides,
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
    await act(async () => button('Delete chat…').click());
    expect(p.onDelete).not.toHaveBeenCalled();
    expect(container.textContent).toContain('Pi’s saved session on Ubuntu is not deleted');
    await act(async () => button('Delete from this Mac').click());
    expect(p.onDelete).toHaveBeenCalledWith('thread');
  });
  it('renders saved turns with incomplete status and never loads model-supplied images or links', async () => {
    await render(props({ messages: [{ id: 'answer', role: 'assistant', text: '![track](https://example.invalid/pixel) [click](https://example.invalid/) **On**', status: 'cancelled' }] }));
    expect(container.querySelector('img')).toBeNull(); expect(container.querySelector('a')).toBeNull();
    expect(container.textContent).toContain('Stopped. This response may be incomplete.');
    expect(container.querySelector('strong')?.textContent).toBe('On');
  });
});
