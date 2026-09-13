import { describe, expect, it } from 'vitest';
import { assistantError, isAssistantConversation, isConversationId } from './assistant';
const id = '01234567-89ab-4cde-8fab-0123456789ab';
describe('assistant native boundary', () => {
  it('accepts native conversation shape without inventing summary-only fields', () => {
    expect(isAssistantConversation({ id, title: 'New chat', createdAtMs: 1, updatedAtMs: 1, messages: [], activePassId: null })).toBe(true);
  });
  it('rejects paths, invalid states, malformed content and pass IDs', () => {
    expect(isConversationId('/etc/passwd')).toBe(false);
    expect(isConversationId('../' + id)).toBe(false);
    expect(isAssistantConversation({ id, title: 'x', createdAtMs: 1, updatedAtMs: 1, messages: [], activePassId: -1 })).toBe(false);
    expect(isAssistantConversation({ id, title: 'x', createdAtMs: 1, updatedAtMs: 1, activePassId: null, messages: [{ id, requestId: id, role: 'system', content: 'bad', createdAtMs: 1, status: 'ready', errorCode: null }] })).toBe(false);
  });
  it('never forwards arbitrary native errors or credential-looking exception text', () => {
    expect(assistantError('secret-example-token')).not.toContain('secret-example-token');
    expect(assistantError('query_busy')).toContain('Another query');
  });
});
