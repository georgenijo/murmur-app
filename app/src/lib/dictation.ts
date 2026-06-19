import { invoke } from '@tauri-apps/api/core';
import { DEFAULT_SETTINGS, Settings, AppProfile } from './settings';

export interface DictationResponse {
  type: string;
  state?: string;
  text?: string;
  model?: string;
  error?: string;
  /** Decoded audio length in seconds (file transcription only). */
  duration?: number;
}

export async function initDictation(): Promise<DictationResponse> {
  return await invoke('init_dictation');
}

export async function startRecording(deviceName?: string): Promise<DictationResponse> {
  try {
    return await invoke('start_native_recording', {
      deviceName: deviceName && deviceName !== DEFAULT_SETTINGS.microphone ? deviceName : null,
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

export async function cancelRecording(): Promise<void> {
  await invoke('cancel_native_recording');
}

export async function getStatus(): Promise<DictationResponse> {
  return await invoke('get_status');
}

export interface ConfigureOptions {
  model?: string;
  language?: string;
  autoPaste?: boolean;
  autoPasteDelayMs?: number;
  vadSensitivity?: number;
  idleTimeoutMinutes?: number;
  customVocabulary?: string;
  smartPunctuation?: boolean;
  saveTranscript?: boolean;
  saveAudio?: boolean;
  outputDir?: string;
  appProfiles?: AppProfile[];
  voiceCommandsEnabled?: boolean;
  cleanupEnabled?: boolean;
  codeVocabEnabled?: boolean;
  codeVocabFolder?: string;
}

export async function configure(options: ConfigureOptions): Promise<DictationResponse> {
  return await invoke('configure_dictation', { options });
}

/**
 * Extract only the backend-configurable fields from a Settings object. Keeps
 * UI-only fields (disabled, launchAtLogin, recordingMode, microphone, etc.) out
 * of the `configure_dictation` payload so callers can't accidentally send them.
 */
export function buildConfigureOptions(s: Settings): ConfigureOptions {
  return {
    model: s.model,
    language: s.language,
    autoPaste: s.autoPaste,
    autoPasteDelayMs: s.autoPasteDelayMs,
    vadSensitivity: s.vadSensitivity,
    idleTimeoutMinutes: s.idleTimeoutMinutes,
    customVocabulary: s.customVocabulary,
    smartPunctuation: s.smartPunctuation,
    saveTranscript: s.saveTranscript,
    saveAudio: s.saveAudio,
    outputDir: s.outputDir,
    appProfiles: s.appProfiles,
    voiceCommandsEnabled: s.voiceCommandsEnabled,
    cleanupEnabled: s.cleanupEnabled,
    codeVocabEnabled: s.codeVocabEnabled,
    codeVocabFolder: s.codeVocabFolder,
  };
}

export async function countVocabTokens(text: string): Promise<number | null> {
  return await invoke('count_vocab_tokens', { text });
}

/**
 * Transcribe an existing audio file (WAV/MP3/M4A) via the Rust `transcribe_file`
 * command. Unlike live recording this returns the text directly (no auto-paste).
 */
export async function transcribeFile(filePath: string): Promise<DictationResponse> {
  try {
    return await invoke('transcribe_file', { filePath });
  } catch (err) {
    return {
      type: 'error',
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

/**
 * Reset this app's stale macOS Accessibility TCC entry, then reopen the
 * Accessibility settings pane. Rejects if the reset fails (see Rust
 * `reset_accessibility_permission`).
 */
export async function resetAccessibilityPermission(): Promise<void> {
  return await invoke('reset_accessibility_permission');
}

/** Live microphone authorization state, mirroring the Rust banner-state mapping. */
export type MicPermissionStatus = 'granted' | 'denied' | 'notDetermined' | 'unknown';

/**
 * Query the *current* microphone authorization status (never cached). Unlike the
 * boolean `check_microphone_permission`, this preserves the distinction between a
 * genuine "denied" and a transient "notDetermined"/"unknown" state, so the banner
 * doesn't false-negative after a dev rebuild or app move (issue #190).
 */
export async function checkMicrophonePermissionStatus(): Promise<MicPermissionStatus> {
  return await invoke('check_microphone_permission_status');
}

/**
 * Reset this app's stale macOS Microphone TCC entry, then reopen the Microphone
 * settings pane. Rejects if the reset fails (see Rust `reset_microphone_permission`).
 */
export async function resetMicrophonePermission(): Promise<void> {
  return await invoke('reset_microphone_permission');
}
