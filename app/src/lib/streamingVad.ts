/**
 * Streaming speech detection (workstream E1): the optional Silero VAD model.
 *
 * The model (2.3 MB, pinned and hash-verified) is only ever fetched by a user
 * action: switching the setting on or pressing Download/Retry. Starting a
 * recording never downloads it; without the model the batch VAD runs.
 */
import { invoke } from '@tauri-apps/api/core';

/** Pinned silero_vad.onnx. */
export const STREAMING_VAD_MODEL_SIZE_LABEL = '2.3 MB';

export type StreamingVadModelState = 'notDownloaded' | 'downloading' | 'ready' | 'failed';

/** Mirrors `ModelStatus` in `vad_stream.rs`. */
export interface StreamingVadModelStatus {
  state: StreamingVadModelState;
  sizeBytes: number;
  /** Present only while `state` is `failed`. */
  error?: string | null;
}

export function streamingVadModelStatus(): Promise<StreamingVadModelStatus> {
  return invoke('streaming_vad_model_status');
}

export function downloadStreamingVadModel(): Promise<void> {
  return invoke('download_streaming_vad_model');
}

export function removeStreamingVadModel(): Promise<void> {
  return invoke('remove_streaming_vad_model');
}
