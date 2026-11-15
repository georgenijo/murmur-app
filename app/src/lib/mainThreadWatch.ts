import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { isNonNegativeInteger, isRecord } from './typeGuards';

export interface MainThreadWatchStatus {
  /** The persisted Diagnostics setting. */
  setting: boolean;
  /** The watch is running now. */
  running: boolean;
  /** `MURMUR_MAIN_THREAD_WATCH` arms it, so the setting cannot turn it off. */
  forcedByEnv: boolean;
  /** Grows with every change. A view keeps the newest status it has seen. */
  seq: number;
}

export const MAIN_THREAD_WATCH_STATUS_EVENT = 'main-thread-watch-status';

function isMainThreadWatchStatus(value: unknown): value is MainThreadWatchStatus {
  return isRecord(value)
    && typeof value.setting === 'boolean'
    && typeof value.running === 'boolean'
    && typeof value.forcedByEnv === 'boolean'
    && isNonNegativeInteger(value.seq);
}

function parseStatus(value: unknown): MainThreadWatchStatus {
  if (!isMainThreadWatchStatus(value)) {
    throw new Error('Murmur returned an unsupported main-thread watch status.');
  }
  return value;
}

export async function getMainThreadWatchStatus(): Promise<MainThreadWatchStatus> {
  return parseStatus(await invoke<unknown>('get_main_thread_watch_status'));
}

export async function setMainThreadWatchArmed(armed: boolean): Promise<MainThreadWatchStatus> {
  return parseStatus(await invoke<unknown>('set_main_thread_watch_armed', { armed }));
}

/** Hear every change, from any Diagnostics view. Subscribe before the first read. */
export function listenMainThreadWatchStatus(
  onStatus: (status: MainThreadWatchStatus) => void,
): Promise<UnlistenFn> {
  return listen<unknown>(MAIN_THREAD_WATCH_STATUS_EVENT, event => {
    if (isMainThreadWatchStatus(event.payload)) onStatus(event.payload);
  });
}
