import { invoke } from '@tauri-apps/api/core';
import type { TransformKey } from './settings';

/** Transform model lifecycle status from `transform_model_status`. */
export type TransformModelState = 'notDownloaded' | 'downloading' | 'ready';

export interface TransformModelStatus {
  state: TransformModelState;
  path: string | null;
  sizeBytes: number;
  sha256: string;
}

export interface TransformModelDownloadProgress {
  downloadedBytes: number;
  totalBytes: number;
}

export async function transformModelStatus(): Promise<TransformModelStatus> {
  return invoke('transform_model_status');
}

export async function downloadTransformModel(): Promise<void> {
  return invoke('download_transform_model');
}

export async function removeTransformModel(): Promise<void> {
  return invoke('remove_transform_model');
}

export async function resetTransformRuntime(): Promise<void> {
  return invoke('reset_transform_runtime');
}

/** Rejected if `hotkey` collides with a dictation hold key. */
export async function setTransformKey(hotkey: TransformKey | null): Promise<void> {
  if (hotkey === null) {
    await invoke('stop_transform_listener');
    return;
  }
  await invoke('set_transform_key', { hotkey });
}

export async function startTransformListener(hotkey: TransformKey): Promise<void> {
  await invoke('start_transform_listener', { hotkey });
}

export async function stopTransformListener(): Promise<void> {
  await invoke('stop_transform_listener');
}

/** ~1.1 GB pinned Qwen2.5-1.5B GGUF (exact size from the catalog pin). */
export const TRANSFORM_MODEL_SIZE_LABEL = '~1.1 GB';
