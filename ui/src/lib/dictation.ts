import { invoke } from '@tauri-apps/api/core';

export type DictationState = 'idle' | 'recording' | 'processing';

export interface DictationResponse {
  type: string;
  state?: DictationState;
  text?: string;
  raw_text?: string;
  message?: string;
  code?: string;
  model?: string;
  backend?: string;
}

export async function initDictation(): Promise<DictationResponse> {
  return invoke('init_dictation');
}

export async function startRecording(): Promise<DictationResponse> {
  return invoke('start_recording');
}

export async function stopRecording(): Promise<DictationResponse> {
  return invoke('stop_recording');
}

export async function getStatus(): Promise<DictationResponse> {
  return invoke('get_status');
}

export interface ConfigureOptions {
  model?: string;
  language?: string;
}

export async function configure(options: ConfigureOptions): Promise<DictationResponse> {
  return invoke('configure_dictation', { options });
}
