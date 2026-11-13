import { invoke } from '@tauri-apps/api/core';
import { microphoneDeviceNameArg, Settings, AppProfile, VocabularyEntry, type SmartAutoMicrophoneRequest } from './settings';

export interface DictationResponse {
  type: string;
  state?: string;
  text?: string;
  model?: string;
  error?: string;
  /** Decoded audio length in seconds (file transcription only). */
  duration?: number;
  /** Generation this start created (`recording_starting` only). */
  recordingId?: number;
}

export async function initDictation(): Promise<DictationResponse> {
  return await invoke('init_dictation');
}

export async function startRecording(
  deviceName?: string,
  origin: 'toggle' | 'hold' = 'toggle',
  smartAuto?: SmartAutoMicrophoneRequest | null,
  gestureToken?: string,
): Promise<DictationResponse> {
  try {
    return await invoke('start_native_recording', {
      deviceName: smartAuto ? null : microphoneDeviceNameArg(deviceName),
      smartAuto: smartAuto ?? null,
      origin,
      ...(gestureToken === undefined ? {} : { gestureToken }),
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

/**
 * Mirror the dictation microphone selection natively for Fn/Globe key-down
 * capture, using the exact arguments `startRecording` sends, so the audio
 * opened at key-down can be adopted by the confirmed start.
 */
export async function syncFnCaptureInput(
  deviceName?: string,
  smartAuto?: SmartAutoMicrophoneRequest | null,
): Promise<void> {
  await invoke('set_fn_capture_input', {
    deviceName: smartAuto ? null : microphoneDeviceNameArg(deviceName),
    smartAuto: smartAuto ?? null,
  });
}

export type DictationStopTrigger = 'hold_release' | 'toggle' | 'overlay' | 'auto_stop' | 'other';

export async function stopRecording(stopTrigger: DictationStopTrigger = 'other'): Promise<DictationResponse> {
  try {
    // Native recording handles transcription and returns the result directly
    return await invoke('stop_native_recording', { stopTrigger });
  } catch (err) {
    return {
      type: 'error',
      error: err instanceof Error ? err.message : String(err)
    };
  }
}

/** With `expectedRecordingId`, cancels only that generation (no-op otherwise). */
export async function cancelRecording(expectedRecordingId?: number): Promise<void> {
  if (expectedRecordingId === undefined) {
    await invoke('cancel_native_recording');
  } else {
    await invoke('cancel_native_recording', { expectedRecordingId });
  }
}

export async function getStatus(): Promise<DictationResponse> {
  return await invoke('get_status');
}

export async function isNotchPillInstalled(): Promise<boolean> {
  return await invoke('is_notchpill_installed');
}

export interface ConfigureOptions {
  model?: string;
  language?: string;
  autoPaste?: boolean;
  smartDestinationEnabled?: boolean;
  autoPasteDelayMs?: number;
  vadSensitivity?: number;
  idleTimeoutMinutes?: number;
  captureStandbyEnabled?: boolean;
  captureStandbySeconds?: number;
  capturePreRollMs?: number;
  customVocabulary?: string;
  vocabularyEntries?: VocabularyEntry[];
  smartPunctuation?: boolean;
  saveTranscript?: boolean;
  saveAudio?: boolean;
  mirrorToNotchPill?: boolean;
  outputDir?: string;
  appProfiles?: AppProfile[];
  voiceCommandsEnabled?: boolean;
  cleanupEnabled?: boolean;
  smartFormattingEnabled?: boolean;
  cleanupRemoveFiller?: boolean;
  cleanupCapitalize?: boolean;
  codeVocabEnabled?: boolean;
  codeVocabFolder?: string;
  correctionEnabled?: boolean;
  correctionFuzzy?: boolean;
  homophoneContextEnabled?: boolean;
  streamingPipelineEnabled?: boolean;
  muteSpeakersWhileDictating?: boolean;
}

// Settings sends full snapshots; other callers may send partial updates. Keep
// requests from this webview in order so an older edit cannot restore a prior
// vocabulary matcher. Settle UI confirmation or rollback before the next job.
let lastConfigure: Promise<void> = Promise.resolve();

export interface ConfigureSettlement {
  onSuccess?: () => void;
  onError?: () => void;
}

export function configure(
  options: ConfigureOptions | (() => ConfigureOptions),
  settlement?: ConfigureSettlement,
): Promise<DictationResponse> {
  const result = lastConfigure.then(async () => {
    try {
      const response = await invoke<DictationResponse>('configure_dictation', {
        options: typeof options === 'function' ? options() : options,
      });
      settlement?.onSuccess?.();
      return response;
    } catch (error) {
      settlement?.onError?.();
      throw error;
    }
  });
  lastConfigure = result.then(() => {}, () => {});
  return result;
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
    smartDestinationEnabled: s.smartDestinationEnabled,
    autoPasteDelayMs: s.autoPasteDelayMs,
    mirrorToNotchPill: s.mirrorToNotchPill,
    vadSensitivity: s.vadSensitivity,
    idleTimeoutMinutes: s.idleTimeoutMinutes,
    captureStandbyEnabled: s.captureStandbyEnabled,
    captureStandbySeconds: s.captureStandbySeconds,
    capturePreRollMs: s.capturePreRollMs,
    customVocabulary: s.customVocabulary,
    vocabularyEntries: s.vocabularyEntries,
    smartPunctuation: s.smartPunctuation,
    saveTranscript: s.saveTranscript,
    saveAudio: s.saveAudio,
    outputDir: s.outputDir,
    appProfiles: s.appProfiles,
    voiceCommandsEnabled: s.voiceCommandsEnabled,
    cleanupEnabled: s.cleanupEnabled,
    smartFormattingEnabled: s.smartFormattingEnabled,
    cleanupRemoveFiller: s.cleanupRemoveFiller,
    cleanupCapitalize: s.cleanupCapitalize,
    codeVocabEnabled: s.codeVocabEnabled,
    codeVocabFolder: s.codeVocabFolder,
    correctionEnabled: s.correctionEnabled,
    correctionFuzzy: s.correctionFuzzy,
    homophoneContextEnabled: s.homophoneContextEnabled,
    streamingPipelineEnabled: s.streamingPipelineEnabled,
    muteSpeakersWhileDictating: s.muteSpeakersWhileDictating,
  };
}

export async function previewVocabularyAliases(
  entries: VocabularyEntry[],
  text: string,
  cliFormatting: boolean,
): Promise<string> {
  return await invoke('preview_vocabulary_aliases', {
    entries,
    text,
    cliFormatting,
  });
}

/** Download a local transcription model by its catalog name. */
export async function downloadModel(modelName: string, requestId?: string): Promise<void> {
  await invoke('download_model', { modelName, ...(requestId ? { requestId } : {}) });
}

/** Resolves only when this owner's native install has stopped. */
export async function cancelModelDownload(modelName: string, requestId: string): Promise<void> {
  await invoke('cancel_model_download', { modelName, requestId });
}

/**
 * Transcribe an existing audio or video file (WAV/MP3/M4A/MP4/MOV) via the Rust `transcribe_file`
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
 * What an Accessibility request did. Exactly one surface opens:
 * - `granted`: already trusted, nothing shown.
 * - `prompted`: the macOS dialog is showing; its own button opens System Settings.
 * - `opened_settings`: no dialog appeared, so the Settings pane was opened.
 * - `restarting`: a stale entry (or a reset) needs a new process for macOS to
 *   list Murmur again; Murmur restarts when idle and asks once it is back.
 * - `restart_deferred`: same stale entry, but Murmur is recording, in a meeting,
 *   or transcribing, so it only opened the pane and did not restart.
 * - `restart_pending`: the reset is done and Murmur restarts as soon as the
 *   current recording, meeting, transcription, download or update finishes.
 * - `in_progress`: an earlier request is still running; nothing new was shown.
 */
export type AccessibilityRequestOutcome =
  | 'granted'
  | 'prompted'
  | 'opened_settings'
  | 'restarting'
  | 'restart_deferred'
  | 'restart_pending'
  | 'in_progress';

/**
 * Reset this app's stale macOS Accessibility TCC entry and bring up a fresh one
 * (see Rust `reset_accessibility_permission`). Rejects if the reset fails or
 * Murmur is busy (recording, meeting, transcription) or not an installed app.
 */
export async function resetAccessibilityPermission(): Promise<AccessibilityRequestOutcome> {
  return await invoke('reset_accessibility_permission');
}

/**
 * Whether Accessibility is on but this running copy of Murmur cannot use it yet:
 * a real Accessibility query from this process failed with "API disabled" while
 * trusted, and none has succeeded since. The cure is a restart (see
 * `restartMurmurForPermission`); nothing restarts on its own.
 */
export async function checkAccessibilityRestartNeeded(): Promise<boolean> {
  return await invoke('check_accessibility_restart_needed');
}

/**
 * Restart Murmur so it picks up an Accessibility grant. Resolves `restarting`;
 * rejects with the reason while Murmur is recording, in a meeting, transcribing,
 * downloading or updating, or when it is not running from an installed app.
 */
export async function restartMurmurForPermission(): Promise<'restarting'> {
  return await invoke('restart_murmur_for_permission');
}

/** Check whether accessibility permission is currently granted (macOS). */
export async function checkAccessibilityPermission(): Promise<boolean> {
  return await invoke('check_accessibility_permission');
}

/**
 * Trigger the accessibility permission flow in one step: clears a stale entry,
 * registers the app in the Accessibility list, and shows either the system dialog
 * or the settings pane, never both.
 */
export async function requestAccessibilityPermission(): Promise<AccessibilityRequestOutcome> {
  return await invoke('request_accessibility_permission');
}

/**
 * Fire the native microphone TCC prompt in-app (no device open, no audio
 * ducking). Fire-and-forget: poll `checkMicrophonePermissionStatus` to observe
 * the user's answer.
 */
export async function requestMicrophoneAccess(): Promise<void> {
  return await invoke('request_microphone_access');
}

/** Open System Settings at the Microphone privacy pane. */
export async function openMicrophoneSettings(): Promise<void> {
  return await invoke('request_microphone_permission');
}

/** Check whether a specific model's files already exist on disk. */
export async function checkModelExists(modelName: string): Promise<boolean> {
  return await invoke('check_specific_model_exists', { modelName });
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
 * Reset this app's stale macOS Microphone TCC entry and relaunch Murmur so the
 * reset takes effect (a running process keeps reporting the old state). The
 * next dictation then asks macOS once; the settings pane is not opened. Resolves
 * `restarting`; rejects if the reset fails or Murmur is busy (recording, meeting,
 * transcription) or not an installed app (see Rust `reset_microphone_permission`).
 */
export async function resetMicrophonePermission(): Promise<'restarting' | 'restart_pending'> {
  return await invoke('reset_microphone_permission');
}
