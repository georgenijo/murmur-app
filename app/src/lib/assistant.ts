import { invoke } from '@tauri-apps/api/core';
import type { QueryCommandConfig } from './queryProviders';

export interface AssistantSummary { id: string; title: string; createdAtMs: number; updatedAtMs: number; messageCount: number }
export type AssistantActionStatus = 'proposed' | 'cancelled' | 'expired' | 'executing' | 'completed' | 'failed' | 'uncertain';
export interface AssistantAction {
  schema_version: 1;
  action_id: string;
  idempotency_key: string;
  kind: 'lights.set';
  actor_id: string;
  connection_id: string;
  conversation_id: string;
  request_id: string;
  targets: Array<{ entity_id: string; name: string | null }>;
  parameters: { power: 'on' | 'off'; brightness_pct: number | null; rgb_color: [number, number, number] | null };
  created_at: string;
  expires_at: string;
  parameter_digest: string;
  status: AssistantActionStatus;
  receipt: null | { execution_id: string; started_at: string; finished_at: string | null; write_attempted: boolean };
  verification: null | {
    source: 'home_assistant_reported'; verified_at: string; matched: boolean | null; message: string;
    states: Array<{ entity_id: string; name: string | null; state: 'on' | 'off' | 'unknown' | 'unavailable' }>;
  };
}
export interface AssistantMessage {
  id: string; requestId: string; role: 'user' | 'assistant'; content: string; createdAtMs: number;
  status: 'pending' | 'running' | 'ready' | 'failed' | 'cancelled' | 'interrupted'; errorCode: string | null;
  actions: AssistantAction[];
}
export interface AssistantConversation extends Omit<AssistantSummary, 'messageCount'> {
  messages: AssistantMessage[];
  activePassId: number | null;
  liveState?: string | null;
  confirmationOutcomeUnknownActionIds: string[];
}
export interface AssistantReceipt { queryPassId: number; conversationId: string; requestId: string }
export const isConversationId = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value);
const isCanonicalId = (value: unknown): value is string => isConversationId(value) && value === value.toLowerCase();
const isDigest = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{64}$/.test(value);
const isRecord = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const finiteTime = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
const isNullableString = (value: unknown): value is string | null => value === null || typeof value === 'string';
const isRgb = (value: unknown): value is [number, number, number] => Array.isArray(value) && value.length === 3 && value.every((part) => Number.isInteger(part) && part >= 0 && part <= 255);
const isActionStatus = (value: unknown): value is AssistantActionStatus => typeof value === 'string' && ['proposed', 'cancelled', 'expired', 'executing', 'completed', 'failed', 'uncertain'].includes(value);
function isAssistantAction(value: unknown): value is AssistantAction {
  if (!isRecord(value) || value.schema_version !== 1 || !isCanonicalId(value.action_id) || !isDigest(value.idempotency_key)
    || value.kind !== 'lights.set' || typeof value.actor_id !== 'string' || !isCanonicalId(value.connection_id)
    || !isCanonicalId(value.conversation_id) || !isCanonicalId(value.request_id) || !isDigest(value.parameter_digest)
    || typeof value.created_at !== 'string' || typeof value.expires_at !== 'string' || !isActionStatus(value.status)) return false;
  if (!Array.isArray(value.targets) || value.targets.length < 1 || value.targets.length > 16 || !value.targets.every((target) => (
    isRecord(target) && typeof target.entity_id === 'string' && /^light\.[a-z0-9_]+$/.test(target.entity_id) && isNullableString(target.name)
  ))) return false;
  if (!isRecord(value.parameters) || !['on', 'off'].includes(String(value.parameters.power))
    || !(value.parameters.brightness_pct === null || (Number.isInteger(value.parameters.brightness_pct) && Number(value.parameters.brightness_pct) >= 1 && Number(value.parameters.brightness_pct) <= 100))
    || !(value.parameters.rgb_color === null || isRgb(value.parameters.rgb_color))) return false;
  if (!(value.receipt === null || (isRecord(value.receipt) && isCanonicalId(value.receipt.execution_id)
    && typeof value.receipt.started_at === 'string' && isNullableString(value.receipt.finished_at) && typeof value.receipt.write_attempted === 'boolean'))) return false;
  return value.verification === null || (isRecord(value.verification) && value.verification.source === 'home_assistant_reported'
    && typeof value.verification.verified_at === 'string' && (value.verification.matched === null || typeof value.verification.matched === 'boolean')
    && typeof value.verification.message === 'string' && Array.isArray(value.verification.states) && value.verification.states.every((state) => (
      isRecord(state) && typeof state.entity_id === 'string' && isNullableString(state.name) && ['on', 'off', 'unknown', 'unavailable'].includes(String(state.state))
    )));
}
export function isAssistantSummary(value: unknown): value is AssistantSummary {
  return isRecord(value) && isConversationId(value.id) && typeof value.title === 'string' && value.title.length <= 512
    && finiteTime(value.createdAtMs) && finiteTime(value.updatedAtMs) && finiteTime(value.messageCount);
}
export function isAssistantConversation(value: unknown): value is AssistantConversation {
  if (!(isRecord(value) && isConversationId(value.id) && typeof value.title === 'string' && value.title.length <= 512 && finiteTime(value.createdAtMs) && finiteTime(value.updatedAtMs)
    && (value.activePassId === null || (finiteTime(value.activePassId) && value.activePassId > 0)) && Array.isArray(value.messages) && value.messages.length <= 200 && value.messages.every((message) => (
    isRecord(message) && typeof message.id === 'string' && isConversationId(message.requestId) && ['user', 'assistant'].includes(String(message.role))
    && typeof message.content === 'string' && message.content.length <= 2 * 1024 * 1024 && finiteTime(message.createdAtMs)
    && ['pending', 'running', 'ready', 'failed', 'cancelled', 'interrupted'].includes(String(message.status))
    && (message.errorCode === null || typeof message.errorCode === 'string') && Array.isArray(message.actions) && message.actions.length <= 16
    && message.actions.every(isAssistantAction)
  )))) return false;
  const messages = value.messages as unknown[];
  const unknown = value.confirmationOutcomeUnknownActionIds;
  return Array.isArray(unknown) && unknown.length <= 16 && unknown.every(isCanonicalId)
    && new Set(unknown).size === unknown.length
    && unknown.every((actionId) => messages.some((message) => (
      isRecord(message) && Array.isArray(message.actions)
      && message.actions.some((action) => isRecord(action) && action.action_id === actionId)
    )));
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
