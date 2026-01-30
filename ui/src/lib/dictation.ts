import { invoke } from '@tauri-apps/api/core';
import { startAudioCapture, stopAudioCapture } from './audioCapture';

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
  try {
    await startAudioCapture();
    return { type: 'ack', state: 'recording' };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    if (message.includes('Permission') || message.includes('NotAllowed')) {
      return {
        type: 'error',
        message: 'Microphone access denied. Please grant permission in System Settings.',
        code: 'MIC_PERMISSION_DENIED',
        state: 'idle',
      };
    }
    return {
      type: 'error',
      message: `Recording failed: ${message}`,
      code: 'RECORDING_FAILED',
      state: 'idle',
    };
  }
}

export async function stopRecording(): Promise<DictationResponse> {
  try {
    const audioBase64 = await stopAudioCapture();
    const response: DictationResponse = await invoke('process_audio', {
      audioData: audioBase64,
    });
    return response;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return {
      type: 'error',
      message: `Processing failed: ${message}`,
      code: 'PROCESSING_FAILED',
      state: 'idle',
    };
  }
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
