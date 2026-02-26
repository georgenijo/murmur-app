import { invoke } from '@tauri-apps/api/core';

export interface DictationResponse {
  type: string;
  state?: string;
  text?: string;
  model?: string;
  error?: string;
}

export async function initDictation(): Promise<DictationResponse> {
  return await invoke('init_dictation');
}

export async function startRecording(deviceName?: string): Promise<DictationResponse> {
  try {
    return await invoke('start_native_recording', {
      deviceName: deviceName && deviceName !== 'system_default' ? deviceName : null,
    });
  } catch (err) {
    const errorMessage = err instanceof Error ? err.message : String(err);

    // Check for common permission errors
    if (errorMessage.includes('permission') || errorMessage.includes('No input device')) {
      return {
        type: 'error',
        error: 'Microphone access denied. Please grant permission in System Settings → Privacy & Security → Microphone'
      };
    }

    return {
      type: 'error',
      error: errorMessage
    };
  }
}

export async function stopRecording(): Promise<DictationResponse> {
  try {
    // Native recording handles transcription and returns the result directly
    return await invoke('stop_native_recording');
  } catch (err) {
    return {
      type: 'error',
      error: err instanceof Error ? err.message : String(err)
    };
  }
}

export async function getStatus(): Promise<DictationResponse> {
  return await invoke('get_status');
}

export interface ConfigureOptions {
  model?: string;
  language?: string;
  autoPaste?: boolean;
}

export async function configure(options: ConfigureOptions): Promise<DictationResponse> {
  return await invoke('configure_dictation', { options });
}
