import { invoke } from '@tauri-apps/api/core';
import { listen } from './targetedEvents';
import { isDictationStatus, type DictationStatus } from './types';
import { flog } from './log';

export interface RecordingSnapshot {
  state: DictationStatus;
  recordingId?: number;
  recordingStartedAtMs?: number | null;
  locked?: boolean;
}

/** Shared by main and overlay: subscribe first, then fence the read by event epoch. */
export function subscribeRecordingStatus(
  applyStatus: (status: DictationStatus) => void,
  applySnapshot: (snapshot: RecordingSnapshot, eventSeen: boolean) => void = ({ state }, eventSeen) => {
    if (!eventSeen) applyStatus(state);
  },
  options: {
    generation?: (recordingId: number) => void;
    locked?: (locked: boolean) => void;
  } = {},
): () => void {
  let disposed = false;
  // Any accepted event after this subscription starts outranks its initial read.
  let epoch = 0;
  let lockEventSeen = false;
  const stops: (() => void)[] = [];
  const subscriptions: Promise<void>[] = [];
  const subscribe = <T>(name: string, apply: (payload: T) => boolean) => {
    subscriptions.push(listen<T>(name, ({ payload }) => {
      if (!disposed && apply(payload)) epoch += 1;
    }).then(stop => { if (disposed) stop(); else stops.push(stop); }));
  };
  subscribe<unknown>('recording-status-changed', payload => {
    if (!isDictationStatus(payload)) return false;
    applyStatus(payload);
    return true;
  });
  if (options.generation) subscribe<{ recordingId?: number }>('dictation-generation-started', payload => {
    const id = payload?.recordingId;
    if (typeof id !== 'number' || !Number.isSafeInteger(id) || id <= 0) return false;
    options.generation!(id);
    return true;
  });
  if (options.locked) subscribe<unknown>('dictation-lock-changed', payload => {
    if (typeof payload !== 'boolean') return false;
    lockEventSeen = true;
    options.locked!(payload);
    return false;
  });
  void Promise.all(subscriptions).then(async () => {
    if (disposed) return;
    // Include events delivered while subscription promises were settling. A
    // late snapshot must never overwrite even a same-generation terminal event.
    const snapshot = await invoke<RecordingSnapshot>('get_status');
    if (disposed) return;
    if (!snapshot || !isDictationStatus(snapshot.state)) throw new Error('Invalid recording status snapshot');
    // A lock-only event must not suppress status hydration. Preserve its newer
    // lock value independently of the lifecycle snapshot.
    applySnapshot({ ...snapshot, locked: lockEventSeen ? undefined : snapshot.locked }, epoch !== 0);
  }).catch(() => {
    if (!disposed) flog.warn('recording', 'recording status synchronization failed');
  });
  return () => { disposed = true; stops.forEach(stop => stop()); };
}
