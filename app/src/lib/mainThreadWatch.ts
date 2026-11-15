import { invoke } from '@tauri-apps/api/core';
import { isRecord } from './typeGuards';

export interface MainThreadWatchStatus {
  /** The persisted Diagnostics setting. */
  setting: boolean;
  /** The watch is running now. */
  running: boolean;
  /** `MURMUR_MAIN_THREAD_WATCH` arms it, so the setting cannot turn it off. */
  forcedByEnv: boolean;
}

function isMainThreadWatchStatus(value: unknown): value is MainThreadWatchStatus {
  return isRecord(value)
    && typeof value.setting === 'boolean'
    && typeof value.running === 'boolean'
    && typeof value.forcedByEnv === 'boolean';
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
