import { PRESENTATION_KEYS, PRESENTATION_CHANGED, ownsPresentationKey, presentationStorage, setPresentationReloader, cacheFailure } from './presentationStorage';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, sendTo } from './targetedEvents';

// This cache contains presentation source state only. Migration ownership and
// validation belong to murmur-core; malformed raw strings reach Rust intact.
export const V2_KEYS = PRESENTATION_KEYS;
const LEGACY_KEYS = ['dictation-history', 'dictation-stats', 'murmur-benchmark-report', 'murmur-benchmark-reports', ...V2_KEYS];
interface StateReceipt { v1: boolean; v2: boolean; values: Record<string, unknown> }
export const SOURCE_STORAGE_FAILED = 'murmur-source-storage-error';
export const SOURCE_STORAGE_SAVED = 'murmur-source-storage-saved';
const inFlight = new Map<number, Promise<void>>();
let generation = 0;
export type OwnedEdits = Partial<Record<'autoPaste' | 'disabled', boolean>>;
type OwnedField = keyof OwnedEdits;
export type OwnedCommit = { autoPaste?: boolean; disabled?: boolean; ownedFieldsRevision?: number; intentGeneration?: number; field?: OwnedField; acknowledgedFields?: OwnedField[] };
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
  if (previous?.raw === raw && JSON.stringify(previous.ownedEdits ?? {}) === JSON.stringify({ ...previous.ownedEdits, ...ownedEdits })) return;
  const nextGeneration = ++generation;
  const fieldGenerations = { ...previous?.fieldGenerations };
  for (const field of ['autoPaste', 'disabled'] as const) {
    if (typeof ownedEdits?.[field] === 'boolean') fieldGenerations[field] = nextGeneration;
  }
  const intent = { raw, generation: nextGeneration, ownedEdits: { ...previous?.ownedEdits, ...ownedEdits }, fieldGenerations };
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
  try {
    request = key === 'dictation-settings' || key === 'murmur-theme-library'
      ? invoke(key === 'dictation-settings' ? 'save_settings_blob' : 'save_theme_library_blob', { blob: intent.raw, ...(wireEdits ? { ownedEdits: wireEdits } : {}), ...(key === 'dictation-settings' ? { intentGeneration: intent.generation } : {}) })
      : invoke('save_webkit_source', { key, value: JSON.parse(intent.raw) as unknown });
  } catch { request = Promise.reject(new Error('Source storage unavailable')); }
  const promise = Promise.resolve(request).then(async () => {
    const current = pending.get(key);
    if (current) {
      const ownedEdits = { ...current.ownedEdits };
      const fieldGenerations = { ...current.fieldGenerations };
      for (const field of ['autoPaste', 'disabled'] as const) {
        if (typeof wireEdits[field] === 'boolean' && fieldGenerations[field] === intent.fieldGenerations?.[field]) {
          delete ownedEdits[field]; delete fieldGenerations[field];
        }
      }
      if (current.generation === intent.generation && !Object.keys(ownedEdits).length) pending.delete(key);
      else pending.set(key, { ...current, ownedEdits, fieldGenerations });
    }
    if (synchronization) await notifySourceCommitted(key);
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
  // One explicit retry pass. A newer rebase/edit discovered during that pass
  // also reaches native ownership before returning.
  const attempted = new Set<number>();
  for (;;) {
    const batch = [...pending].filter(([, intent]) => !attempted.has(intent.generation));
    if (!batch.length) return;
    await Promise.all(batch.map(([key, intent]) => {
      attempted.add(intent.generation);
      return submitIntent(key, intent);
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
    const unacknowledged = settings.intentGeneration !== undefined && settings.acknowledgedFields !== undefined
      && !settings.acknowledgedFields.includes(field);
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
  pending.set('dictation-settings', { ...intent, generation: ++generation, raw: JSON.stringify(value), ownedEdits, fieldGenerations });
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
  if (importLegacy && (!state.v1 || !state.v2)) {
    // Do not catch getItem failures or parse JSON here: either would turn an
    // unreadable/corrupt store into a destructive empty-source receipt.
    const source = Object.fromEntries(LEGACY_KEYS.map(key => [key, localStorage.getItem(key)]));
    state = await invoke<StateReceipt>('import_webkit_state', { source });
  }

  for (const [key, value] of Object.entries(state.values)) {
    presentationStorage.setItem(key, JSON.stringify(value));
  }
  if (state.v2) {
    presentationStorage.removeItem('murmur-overlay-vertical-offset');
    presentationStorage.removeItem('murmur-ui-latency-v1');
  }
  await synchronizePresentation();
  } finally {
    if (importLegacy) {
      const { refreshCompletion, removeImportedKeys } = await import('./completionData');
      try { await removeImportedKeys(await refreshCompletion()); } catch { /* Retry on next startup. */ }
    }
  }
}
