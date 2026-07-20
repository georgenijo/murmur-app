import { invoke } from '@tauri-apps/api/core';

export type KnowledgeKind = 'replacement_rule' | 'vocabulary_term' | 'snippet';
export type KnowledgeProvenance = 'manual' | 'code_scan' | 'learned_correction' | 'import';
export type KnowledgeAvailability = 'ready' | 'recovered' | 'reinitialized' | 'unavailable';
export type VoiceCommandKind = 'text_replacement' | 'snippet';

export interface VoiceCommandMetadata {
  commandType: VoiceCommandKind;
  allowClipboardRead: boolean;
}

export type KnowledgeScope =
  | { kind: 'global' }
  | { kind: 'app'; bundleId: string }
  | { kind: 'project'; bundleId: string; root: string };

export type KnowledgePayload =
  | { kind: 'replacement_rule'; source: string; replacement: string }
  | { kind: 'vocabulary_term'; written: string; aliases: string[] }
  | { kind: 'snippet'; trigger: string; body: string };

export interface KnowledgeEntry {
  id: string;
  payload: KnowledgePayload;
  enabled: boolean;
  scope: KnowledgeScope;
  provenance: KnowledgeProvenance;
  createdAtMs: number;
  updatedAtMs: number;
  revision: number;
  voiceCommand?: VoiceCommandMetadata | null;
}

export interface KnowledgeDraft {
  id?: string;
  expectedRevision?: number;
  payload: KnowledgePayload;
  enabled: boolean;
  scope: KnowledgeScope;
  voiceCommand?: VoiceCommandMetadata | null;
}

export interface KnowledgeListRequest {
  query?: string;
  kind?: KnowledgeKind;
  enabled?: boolean;
  scopeKind?: KnowledgeScope['kind'];
  voiceCommand?: boolean;
  limit?: number;
  offset?: number;
}

export interface VoiceCommandPreviewResponse {
  output: string;
  matched: boolean;
  clipboardRequired: boolean;
  clipboardRead: boolean;
}

export interface KnowledgeListResponse {
  entries: KnowledgeEntry[];
  total: number;
  nextOffset: number | null;
  storeRevision: number;
}

export interface KnowledgeStoreStatus {
  availability: KnowledgeAvailability;
  schemaVersion: number;
  recordCount: number;
  storeRevision: number;
  recoveryAtMs: number | null;
  message: string | null;
}

export interface KnowledgeImportSummary {
  total: number;
  new: number;
  duplicates: number;
  conflicts: number;
}

export interface KnowledgeImportResult {
  imported: number;
  duplicates: number;
  storeRevision: number;
}

export const getKnowledgeStatus = () =>
  invoke<KnowledgeStoreStatus>('get_knowledge_store_status');

export const retryKnowledgeStore = () =>
  invoke<KnowledgeStoreStatus>('retry_knowledge_store');

export const listKnowledge = (request: KnowledgeListRequest) =>
  invoke<KnowledgeListResponse>('list_knowledge', { request });

export const upsertKnowledge = (draft: KnowledgeDraft) =>
  invoke<KnowledgeEntry>('upsert_knowledge', { draft });

export const setKnowledgeEnabled = (entry: KnowledgeEntry, enabled: boolean) =>
  invoke<KnowledgeEntry>('set_knowledge_enabled', {
    id: entry.id,
    enabled,
    expectedRevision: entry.revision,
  });

export const deleteKnowledge = (entry: KnowledgeEntry) =>
  invoke<number>('delete_knowledge', {
    id: entry.id,
    expectedRevision: entry.revision,
  });

export const exportKnowledgeToFile = (path: string) =>
  invoke<number>('export_knowledge_to_file', { path });

export const inspectKnowledgeImport = (path: string) =>
  invoke<KnowledgeImportSummary>('inspect_knowledge_import', { path });

export const importKnowledgeFromFile = (path: string) =>
  invoke<KnowledgeImportResult>('import_knowledge_from_file', { path });

export const deleteAllKnowledge = (expectedRevision: number) =>
  invoke<number>('delete_all_knowledge', { expectedRevision });

export const previewVoiceCommand = (
  draft: KnowledgeDraft,
  text: string,
  readClipboard: boolean,
) => invoke<VoiceCommandPreviewResponse>('preview_voice_command', {
  request: { draft, text, readClipboard },
});

export function payloadTitle(payload: KnowledgePayload): string {
  switch (payload.kind) {
    case 'replacement_rule': return payload.source;
    case 'vocabulary_term': return payload.written;
    case 'snippet': return payload.trigger;
  }
}

export function payloadDetail(payload: KnowledgePayload): string {
  switch (payload.kind) {
    case 'replacement_rule': return payload.replacement;
    case 'vocabulary_term': return payload.aliases.join(', ') || 'No spoken aliases';
    case 'snippet': return payload.body;
  }
}

export function scopeLabel(scope: KnowledgeScope): string {
  switch (scope.kind) {
    case 'global': return 'Global';
    case 'app': return `App · ${scope.bundleId}`;
    case 'project': return `Project · ${scope.bundleId} · ${scope.root}`;
  }
}
