import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';

export type SystemAudioPermissionState = 'unknown' | 'granted' | 'denied' | 'unsupported';
export type MeetingRuntimePhase = 'idle' | 'starting' | 'recording' | 'stopping' | 'processing' | 'failed';
export type MeetingSessionStatus = 'active' | 'complete' | 'interrupted' | 'failed';
export type MeetingSegmentStatus = 'pending' | 'final' | 'failed';
export type MeetingSpeaker = 'me' | 'them';

export interface MeetingRuntimeStatus {
  generation: number;
  sessionId: string | null;
  phase: MeetingRuntimePhase;
  elapsedMs: number;
  microphoneActive: boolean;
  systemAudioActive: boolean;
  errorCode: string | null;
}

export interface MeetingSession {
  id: string;
  startedAtMs: number;
  endedAtMs: number | null;
  status: MeetingSessionStatus;
  modelName: string;
  language: string;
  smartPunctuation: boolean;
  retainAudio: boolean;
  durationMs: number;
  segmentCount: number;
  preview: string;
  errorCode: string | null;
}

export interface MeetingSegment {
  id: number;
  sessionId: string;
  speaker: MeetingSpeaker;
  sequence: number;
  startMs: number;
  endMs: number;
  status: MeetingSegmentStatus;
  text: string;
  audioAvailable: boolean;
  errorCode: string | null;
}

export interface MeetingDetail {
  session: MeetingSession;
  segments: MeetingSegment[];
}

export interface MeetingPage {
  sessions: MeetingSession[];
  total: number;
  offset: number;
  limit: number;
}

export interface StartMeetingOptions {
  microphone: string;
  retainAudio: boolean;
  retentionDays: number;
  maxSessions: number;
}

export const IDLE_MEETING_STATUS: MeetingRuntimeStatus = {
  generation: 0,
  sessionId: null,
  phase: 'idle',
  elapsedMs: 0,
  microphoneActive: false,
  systemAudioActive: false,
  errorCode: null,
};

export async function getMeetingStatus(): Promise<MeetingRuntimeStatus> {
  return invoke('get_meeting_status');
}

export async function startMeeting(options: StartMeetingOptions): Promise<MeetingSession> {
  return invoke('start_meeting', {
    request: {
      deviceName: options.microphone,
      retainAudio: options.retainAudio,
      retentionDays: options.retentionDays === 0 ? null : options.retentionDays,
      maxSessions: options.maxSessions,
    },
  });
}

export async function stopMeeting(): Promise<void> {
  await invoke('stop_meeting');
}

export async function listMeetings(query = '', offset = 0, limit = 50): Promise<MeetingPage> {
  return invoke('list_meetings', { query: query || null, offset, limit });
}

export async function getMeeting(id: string): Promise<MeetingDetail> {
  return invoke('get_meeting', { id });
}

export async function deleteMeeting(id: string): Promise<void> {
  await invoke('delete_meeting', { id });
}

export async function deleteAllMeetings(): Promise<void> {
  await invoke('delete_all_meetings');
}

export async function getSystemAudioPermissionStatus(): Promise<SystemAudioPermissionState> {
  return invoke('get_system_audio_permission_status');
}

export async function requestSystemAudioPermission(): Promise<SystemAudioPermissionState> {
  return invoke('request_system_audio_permission');
}

export async function openSystemAudioPreferences(): Promise<void> {
  await invoke('open_system_audio_preferences');
}

export async function copyMeeting(id: string): Promise<void> {
  const text = await invoke<string>('get_meeting_export_text', { id });
  await navigator.clipboard.writeText(text);
}

export async function saveMeetingExport(id: string, startedAtMs: number): Promise<string | null> {
  const text = await invoke<string>('get_meeting_export_text', { id });
  const stamp = new Date(startedAtMs).toISOString().slice(0, 10);
  const path = await save({
    defaultPath: `Murmur Meeting ${stamp}.txt`,
    filters: [{ name: 'Murmur meeting transcript', extensions: ['txt'] }],
  });
  if (typeof path !== 'string' || path.length === 0) return null;
  await invoke('save_text_export', { path, contents: text });
  return path;
}

export function formatMeetingTimestamp(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, '0')}:${String(remainder).padStart(2, '0')}`
    : `${minutes}:${String(remainder).padStart(2, '0')}`;
}

export function orderedMeetingSegments(segments: MeetingSegment[], limit = 200): MeetingSegment[] {
  return [...new Map(segments.map((segment) => [segment.id, segment])).values()]
    .sort((left, right) => left.startMs - right.startMs || left.id - right.id)
    .slice(-Math.max(1, limit));
}

export function meetingErrorMessage(code: string | null): string | null {
  if (!code) return null;
  const messages: Record<string, string> = {
    unsupported_os: 'Meeting capture requires macOS 14.2 or newer.',
    system_audio_permission_denied: 'System Audio access was denied.',
    microphone_permission_denied: 'Microphone access was denied.',
    microphone_unavailable: 'The selected microphone is unavailable.',
    system_audio_unavailable: 'System Audio capture is unavailable.',
    system_audio_callback_stalled: 'System Audio stopped delivering audio.',
    microphone_callback_stalled: 'The microphone stopped delivering audio.',
    permission_prompt_timeout: 'The permission request timed out.',
    capture_setup_timeout: 'Core Audio stalled while starting capture. Quit other audio capture apps and try again.',
    capture_backlog: 'Audio processing could not keep up safely.',
    capture_stop_timeout: 'The capture worker did not stop cleanly. Murmur forced it to exit.',
    termination_unconfirmed: 'The capture worker could not be confirmed stopped. Restart Murmur before recording again.',
    protocol_error: 'The signed capture worker returned an invalid response.',
    spool_failed: 'A meeting audio chunk could not be stored safely.',
    transcription_failed: 'A meeting transcript chunk could not be transcribed.',
    store_unavailable: 'The local meeting transcript store is unavailable.',
  };
  return messages[code] ?? 'Meeting capture stopped unexpectedly.';
}
