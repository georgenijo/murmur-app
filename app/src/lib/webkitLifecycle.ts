import { PRESENTATION_KEYS, PRESENTATION_CHANGED, ownsPresentationKey, presentationStorage, setPresentationReloader, cacheFailure } from './presentationStorage';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, sendTo } from './targetedEvents';
import { sourceSession, nextSourceWriteVersion, invokeSource } from './sourceWriteVersion';

// This cache contains presentation source state only. Migration ownership and
// validation belong to murmur-core; malformed raw strings reach Rust intact.
export const V2_KEYS = PRESENTATION_KEYS;
export const V3_KEYS = ['updater-last-check', 'pending-update-release-notes'];
const LEGACY_KEYS = ['dictation-history', 'dictation-stats', 'murmur-benchmark-report', 'murmur-benchmark-reports', ...V2_KEYS, ...V3_KEYS];
interface StateReceipt { v1: boolean; v2: boolean; v3?: boolean; values: Record<string, unknown> }
export const SOURCE_STORAGE_FAILED = 'murmur-source-storage-error';
export const SOURCE_STORAGE_SAVED = 'murmur-source-storage-saved';
const inFlight = new Map<number, Promise<void>>();
const nextGeneration = () => nextSourceWriteVersion().intentGeneration;
export type OwnedEdits = Partial<Record<'autoPaste' | 'disabled', boolean>>;
type OwnedField = keyof OwnedEdits;
export type OwnedCommit = { autoPaste?: boolean; disabled?: boolean; ownedFieldsRevision?: number; intentGeneration?: number; intentSession?: string; field?: OwnedField; acknowledgedFields?: OwnedField[] };
const pending = new Map<string, { raw: string; generation: number; ownedEdits?: OwnedEdits; fieldGenerations?: Partial<Record<OwnedField, number>> }>();
const SOURCE_COMMITTED = 'presentation-source-committed';
interface SourceCommitted { key: string }
let synchronization: Promise<unknown> | undefined;
type ReadOwner = { key: string; generation: number; pending: boolean };
// One active snapshot and one coalesced follow-up, bounded to owned keys.
const reloadKeys = new Map<string, ReadOwner>();
let reloadInFlight = false;
function refreshPresentation(keys: readonly string[]): void {
  for (const key of keys) {
    if (!ownsPresentationKey(key)) continue;
    // Invalidate an active read immediately, before its stale result can land.
    reloadKeys.set(key, { key, generation: presentationStorage.beginRead(key), pending: pending.has(key) });
  }
  if (!reloadInFlight && reloadKeys.size) void drainPresentationReloads();
}
async function drainPresentationReloads(): Promise<void> {
  reloadInFlight = true;
  try {
    while (reloadKeys.size) {
      const ownership = [...reloadKeys.values()].map(owner => ({ ...owner, pending: owner.pending || pending.has(owner.key) }));
      reloadKeys.clear();
      try {
        const state = await invoke<StateReceipt>('webkit_state_snapshot');
        let changed = false;
        for (const owner of ownership) {
          // Recheck both read and write ownership after the async boundary. An
          // intent pending at either boundary must keep its optimistic session copy.
          if (owner.pending || pending.has(owner.key)) continue;
          const value = Object.prototype.hasOwnProperty.call(state.values, owner.key) ? JSON.stringify(state.values[owner.key]) : null;
          changed = presentationStorage.rememberIfCurrent(owner.key, value, owner.generation) || changed;
        }
        if (changed) window.dispatchEvent(new Event(PRESENTATION_CHANGED));
      } catch { cacheFailure('reload-failed'); }
    }
  } finally { reloadInFlight = false; }
}
function synchronizePresentation(): Promise<unknown> {
  if (!synchronization) {
    setPresentationReloader(keys => { void refreshPresentation(keys); });
    synchronization = listen<SourceCommitted>(SOURCE_COMMITTED, ({ payload }) => {
      if (payload && typeof payload.key === 'string' && ownsPresentationKey(payload.key)) {
        void refreshPresentation([payload.key]);
      }
    }).catch(() => { synchronization = undefined; });
  }
  return synchronization;
}
async function notifySourceCommitted(key: string): Promise<void> {
  const notice: SourceCommitted = { key };
  await Promise.all([
    sendTo<SourceCommitted>('main', SOURCE_COMMITTED, notice),
    sendTo<SourceCommitted>('overlay', SOURCE_COMMITTED, notice),
    sendTo<SourceCommitted>('dictation-preview', SOURCE_COMMITTED, notice),
    sendTo<SourceCommitted>('diagnostics', SOURCE_COMMITTED, notice),
  ]);
}
/** Ordered, retryable commits. A failed write retains the newest intent for
 * each key; an older async completion cannot discard a newer pending edit. */
export function persistSourceState(key: string, raw: string, ownedEdits?: OwnedEdits): void {
  try { if (!isTauri()) return; } catch { return; }
  const previous = pending.get(key);
  if (previous && inFlight.has(previous.generation) && previous.raw === raw
    && JSON.stringify(previous.ownedEdits ?? {}) === JSON.stringify({ ...previous.ownedEdits, ...ownedEdits })) return;
  const editGeneration = nextGeneration();
  const fieldGenerations = { ...previous?.fieldGenerations };
  for (const field of ['autoPaste', 'disabled'] as const) {
    if (typeof ownedEdits?.[field] === 'boolean') fieldGenerations[field] = editGeneration;
  }
  const intent = { raw, generation: editGeneration, ownedEdits: { ...previous?.ownedEdits, ...ownedEdits }, fieldGenerations };
  pending.set(key, intent);
  // Only this edit's deliberate fields go on the immediate request. Older
  // unacknowledged fields stay retryable, but must not be patched again behind
  // a newer native quick-control commit.
  void submitIntent(key, intent, ownedEdits ?? {}).catch(() => {});
}
function submitIntent(key: string, intent: NonNullable<ReturnType<typeof pending.get>>, freshEdits?: OwnedEdits): Promise<void> {
  const existing = inFlight.get(intent.generation);
  if (existing) return existing;
  // invoke occurs now, before awaiting any previous source publication.
  let request: Promise<unknown>;
  const wireEdits = { ...(freshEdits ?? intent.ownedEdits) };
  const version = { sourceSession, intentGeneration: intent.generation };
  try {
    request = key === 'dictation-settings' || key === 'murmur-theme-library'
      ? invokeSource(key === 'dictation-settings' ? 'save_settings_blob' : 'save_theme_library_blob', { blob: intent.raw, ownedEdits: wireEdits, ownedEditGenerations: Object.fromEntries(Object.keys(wireEdits).map(field => [field, intent.fieldGenerations?.[field as OwnedField]])), ...version })
      : invokeSource('save_webkit_source', { key, value: JSON.parse(intent.raw) as unknown, ...version });
  } catch { request = Promise.reject(new Error('Source storage unavailable')); }
  const promise = Promise.resolve(request).then(async result => {
    const superseded = !!(result && typeof result === 'object' && 'status' in result && result.status === 'superseded');
    // Snapshot and field receipts are independent. A superseded snapshot never
    // acknowledges newer content; native may separately own its exact patches.
    const fieldReceipts = result && typeof result === 'object' && 'ownedPatchGenerations' in result
      ? result.ownedPatchGenerations as Partial<Record<OwnedField, number>> : undefined;
    const current = pending.get(key);
    if (current) {
      const ownedEdits = { ...current.ownedEdits };
      const fieldGenerations = { ...current.fieldGenerations };
      for (const field of ['autoPaste', 'disabled'] as const) {
        if (typeof wireEdits[field] === 'boolean' && fieldGenerations[field] === intent.fieldGenerations?.[field]
          && (fieldReceipts ? fieldReceipts[field] === intent.fieldGenerations?.[field] : !superseded)) {
          delete ownedEdits[field]; delete fieldGenerations[field];
        }
      }
      if (!superseded && current.generation === intent.generation && !Object.keys(ownedEdits).length) pending.delete(key);
      else pending.set(key, { ...current, ownedEdits, fieldGenerations });
    }
    if (!superseded && synchronization) await notifySourceCommitted(key);
    if (!pending.size) window.dispatchEvent(new Event(SOURCE_STORAGE_SAVED));
  }, () => {
    if (pending.has(key)) window.dispatchEvent(new Event(SOURCE_STORAGE_FAILED));
    throw new Error('Source storage unavailable');
  }).finally(() => { inFlight.delete(intent.generation); });
  inFlight.set(intent.generation, promise);
  return promise;
}
export async function flushSourceState(): Promise<void> {
  await Promise.allSettled([...inFlight.values()]);
  // Joining an in-flight save does not mean it carried every retained field.
  // Drain until empty, failing visibly on publication errors. Every actual
  // retry gets a fresh native generation, even when its content is identical.
  while (pending.size) {
    await Promise.all([...pending].map(([key, intent]) => {
      if (inFlight.has(intent.generation)) return submitIntent(key, intent);
      const retry = { ...intent, generation: nextGeneration() };
      pending.set(key, retry);
      return submitIntent(key, retry);
    }));
  }
}
/** Rebase a retained intent after a newer native quick-control commit. */
export function rebasePendingOwnedSettings(settings: OwnedCommit): OwnedCommit {
  const intent = pending.get('dictation-settings');
  if (!intent) return settings;
  const value = JSON.parse(intent.raw) as Record<string, unknown>;
  const rebased = { ...settings };
  const ownedEdits = { ...intent.ownedEdits };
  const fieldGenerations = { ...intent.fieldGenerations };
  for (const field of ['autoPaste', 'disabled'] as const) {
    const otherSession = settings.intentSession !== undefined && settings.intentSession !== sourceSession;
    const unacknowledged = otherSession || (settings.intentGeneration !== undefined && settings.acknowledgedFields !== undefined
      && !settings.acknowledgedFields.includes(field));
    const otherNativeField = settings.intentGeneration === undefined && settings.field !== undefined && settings.field !== field;
    if (typeof ownedEdits[field] === 'boolean' && (unacknowledged || otherNativeField
      || (settings.intentGeneration !== undefined && (fieldGenerations[field] ?? 0) > settings.intentGeneration))) {
      rebased[field] = ownedEdits[field];
    } else if (typeof settings[field] === 'boolean') {
      delete ownedEdits[field];
      delete fieldGenerations[field];
    }
    if (typeof rebased[field] === 'boolean') value[field] = rebased[field];
  }
  value.ownedFieldsRevision = settings.ownedFieldsRevision;
  // Rebase is not a new deliberate edit. Retain per-field intent ownership.
  pending.set('dictation-settings', { ...intent, generation: nextGeneration(), raw: JSON.stringify(value), ownedEdits, fieldGenerations });
  return rebased;
}
export const sourceStorage = {
  getItem: (key: string) => presentationStorage.getItem(key),
  setItem(key: string, value: string) {
    persistSourceState(key, value);
    try { presentationStorage.setItem(key, value); } catch { /* Rust intent remains queued. */ }
  },
};
export async function initializeWebkitState(importLegacy = false): Promise<void> {
  try { if (!isTauri()) return; } catch { return; }
  try {
  let state = await invoke<StateReceipt>('webkit_state_snapshot');
  if (importLegacy && (!state.v1 || !state.v2 || !state.v3)) {
    // Do not catch getItem failures or parse JSON here: either would turn an
    // unreadable/corrupt store into a destructive empty-source receipt.
    const source = Object.fromEntries(LEGACY_KEYS.map(key => [key, localStorage.getItem(key)]));
    state = await invoke<StateReceipt>('import_webkit_state', { source });
  }

  for (const [key, value] of Object.entries(state.values)) {
    if (!V3_KEYS.includes(key)) presentationStorage.setItem(key, JSON.stringify(value));
  }
  if (state.v2) {
    presentationStorage.removeItem('murmur-overlay-vertical-offset');
    presentationStorage.removeItem('murmur-ui-latency-v1');
  }
  if (importLegacy && state.v3) {
    for (const key of V3_KEYS) { try { localStorage.removeItem(key); } catch { /* Disk receipt prevents reimport; removal retries next launch. */ } }
  }
  await synchronizePresentation();
  } finally {
    if (importLegacy) {
      const { refreshCompletion, removeImportedKeys } = await import('./completionData');
      try { await removeImportedKeys(await refreshCompletion()); } catch { /* Retry on next startup. */ }
    }
  }
}
