import { invoke } from '@tauri-apps/api/core';
import type { QueryCommandConfig } from './queryProviders';

export interface AssistantSummary { id: string; title: string; createdAtMs: number; updatedAtMs: number; messageCount: number }
export interface AssistantMessage {
  id: string; requestId: string; role: 'user' | 'assistant'; content: string; createdAtMs: number;
  status: 'pending' | 'running' | 'ready' | 'failed' | 'cancelled' | 'interrupted'; errorCode: string | null;
}
export interface AssistantConversation extends Omit<AssistantSummary, 'messageCount'> { messages: AssistantMessage[]; activePassId: number | null; liveState?: string | null }
export interface AssistantReceipt { queryPassId: number; conversationId: string; requestId: string }
export const isConversationId = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value);
const finiteTime = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
export function isAssistantSummary(value: unknown): value is AssistantSummary {
  if (!value || typeof value !== 'object') return false;
  const v = value as AssistantSummary;
  return isConversationId(v.id) && typeof v.title === 'string' && v.title.length <= 512 && finiteTime(v.createdAtMs) && finiteTime(v.updatedAtMs) && finiteTime(v.messageCount);
}
export function isAssistantConversation(value: unknown): value is AssistantConversation {
  if (!value || typeof value !== 'object') return false;
  const v = value as AssistantConversation;
  return isConversationId(v.id) && typeof v.title === 'string' && v.title.length <= 512 && finiteTime(v.createdAtMs) && finiteTime(v.updatedAtMs)
    && (v.activePassId === null || (finiteTime(v.activePassId) && v.activePassId > 0)) && Array.isArray(v.messages) && v.messages.length <= 200 && v.messages.every((m) => (
    m && typeof m.id === 'string' && isConversationId(m.requestId) && ['user', 'assistant'].includes(m.role)
    && typeof m.content === 'string' && m.content.length <= 2 * 1024 * 1024 && finiteTime(m.createdAtMs)
    && ['pending', 'running', 'ready', 'failed', 'cancelled', 'interrupted'].includes(m.status)
    && (m.errorCode === null || typeof m.errorCode === 'string')
  ));
}
export async function listAssistant(): Promise<AssistantSummary[]> {
  const value = await invoke<unknown>('list_assistant_conversations');
  if (!Array.isArray(value) || value.length > 2000 || !value.every(isAssistantSummary)) throw new Error('invalid_assistant_response');
  return value;
}
export async function getAssistant(id: string): Promise<AssistantConversation> {
  if (!isConversationId(id)) throw new Error('invalid_conversation');
  const value = await invoke<unknown>('get_assistant_conversation', { conversationId: id });
  if (!isAssistantConversation(value) || value.id !== id) throw new Error('invalid_assistant_response');
  return value;
}
export async function createAssistant(): Promise<AssistantConversation> {
  const value = await invoke<unknown>('create_assistant_conversation', { consent: true });
  if (!isAssistantConversation(value)) throw new Error('invalid_assistant_response');
  return value;
}
export async function assistantConnected(command: QueryCommandConfig): Promise<boolean> {
  const value = await invoke<{ enabled: boolean }>('get_assistant_connection', { command });
  if (typeof value?.enabled !== 'boolean') throw new Error('invalid_assistant_response');
  return value.enabled;
}
export function assistantError(error: unknown): string {
  const code = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  if (code === 'assistant_connection_changed') return 'This chat belongs to a different Pi connection. Restore that connection or start a new chat.';
  if (/busy|active|in_progress/.test(code)) return 'Another query is still active. Finish or stop it, then try again.';
  if (/consent|connection|custom|configured/.test(code)) return 'Connect your configured Pi Assistant bridge before sending a message.';
  if (/large|limit|full/.test(code)) return 'This conversation or message has reached its limit. Try a shorter message or a new chat.';
  return 'The assistant could not complete that request. Check the Pi connection and try again.';
}
