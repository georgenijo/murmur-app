import { invoke } from '@tauri-apps/api/core';
import type { SmartAutoMicrophoneRequest } from './settings';
import { save } from '@tauri-apps/plugin-dialog';

export type SystemAudioPermissionState = 'unknown' | 'granted' | 'denied' | 'unsupported';

/**
 * Authorization and capture health are independent. A granted tap on a silent
 * Mac reports `permission: 'granted'` with `audioFlowing: false`, which is a
 * healthy result and not a permission problem.
 */
export interface SystemAudioAccess {
  permission: SystemAudioPermissionState;
  captureReady: boolean;
  audioFlowing: boolean;
  needsRelaunch: boolean;
}
export type MeetingRuntimePhase = 'idle' | 'starting' | 'recording' | 'stopping' | 'processing' | 'failed';
export type MeetingSessionStatus = 'active' | 'complete' | 'interrupted' | 'failed';
export type MeetingSegmentStatus = 'pending' | 'final' | 'failed';
export type MeetingSpeaker = 'me' | 'them';
export type EchoCancellationBypassReason =
  | 'initializationFailed'
  | 'unsupportedFormat'
  | 'renderDiscontinuity'
  | 'processorFailed'
  | 'processingBacklog';
export type MeetingEchoCancellationRuntime =
  | { state: 'off' }
  | { state: 'starting' }
  | { state: 'active' }
  | { state: 'bypassed'; reason: EchoCancellationBypassReason };

export interface MeetingRuntimeStatus {
  generation: number;
  sessionId: string | null;
  phase: MeetingRuntimePhase;
  elapsedMs: number;
  microphoneActive: boolean;
  systemAudioActive: boolean;
  echoCancellation: MeetingEchoCancellationRuntime;
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
  labels: MeetingSpeakerLabels;
  generated: GeneratedMeetingReview | null;
  review: SavedMeetingReview | null;
  activeDocument: MeetingReviewDocumentV1 | null;
  activeOrigin: ActiveReviewOrigin | null;
}

export interface MeetingSpeakerLabels { me: string; them: string }
export interface ReviewText { key: string; text: string; sourceSegmentIds: number[] }
export interface ReviewAction extends ReviewText { owner: string | null; dueDate: string | null }
export interface MeetingReviewDocumentV1 {
  schema: 'murmur.meeting-review.v1';
  summary: ReviewText;
  decisions: ReviewText[];
  actionItems: ReviewAction[];
  openQuestions: ReviewText[];
}
export interface GeneratedMeetingReview { revision: number; document: MeetingReviewDocumentV1 }
export interface SavedMeetingReview {
  revision: number;
  basedOnGeneratedRevision: number | null;
  document: MeetingReviewDocumentV1 | null;
}
export type ActiveReviewOrigin = 'generated' | 'reviewed';
export type MeetingReviewExportFormat = 'markdown' | 'text' | 'json';
export interface EditableReviewText { key: string; text: string }
export interface EditableReviewAction extends EditableReviewText { owner: string | null; dueDate: string | null }
export interface EditableReviewDocument {
  summary: EditableReviewText;
  decisions: EditableReviewText[];
  actionItems: EditableReviewAction[];
  openQuestions: EditableReviewText[];
}
export type ReviewEditBase =
  | { kind: 'labels_only' }
  | { kind: 'generated'; generatedRevision: number }
  | { kind: 'review'; reviewRevision: number };
export interface SaveMeetingReviewRequest {
  sessionId: string;
  expectedReviewRevision: number | null;
  base: ReviewEditBase;
  labels: MeetingSpeakerLabels;
  document: EditableReviewDocument | null;
}

export type MeetingSummaryPhase = 'idle' | 'running' | 'cancelling' | 'complete' | 'failed' | 'cancelled';
export interface MeetingSummaryStatus {
  generation: number;
  sessionId: string | null;
  phase: MeetingSummaryPhase;
  completedChunks: number;
  totalChunks: number;
  elapsedMs: number;
  peakRssMb: number;
  errorCode: string | null;
}

export const IDLE_MEETING_SUMMARY_STATUS: MeetingSummaryStatus = {
  generation: 0, sessionId: null, phase: 'idle', completedChunks: 0,
  totalChunks: 0, elapsedMs: 0, peakRssMb: 0, errorCode: null,
};

export interface MeetingPage {
  sessions: MeetingSession[];
  total: number;
  offset: number;
  limit: number;
}

export interface StartMeetingOptions {
  microphone: string;
  smartAuto?: SmartAutoMicrophoneRequest | null;
  retainAudio: boolean;
  retentionDays: number;
  maxSessions: number;
  echoCancellation: boolean;
}

export const IDLE_MEETING_STATUS: MeetingRuntimeStatus = {
  generation: 0,
  sessionId: null,
  phase: 'idle',
  elapsedMs: 0,
  microphoneActive: false,
  systemAudioActive: false,
  echoCancellation: { state: 'off' },
  errorCode: null,
};

export async function getMeetingStatus(): Promise<MeetingRuntimeStatus> {
  return invoke('get_meeting_status');
}

export async function startMeeting(options: StartMeetingOptions): Promise<MeetingSession> {
  return invoke('start_meeting', {
    request: {
      deviceName: options.smartAuto ? null : options.microphone,
      ...(options.smartAuto ? { smartAuto: options.smartAuto } : {}),
      retainAudio: options.retainAudio,
      retentionDays: options.retentionDays === 0 ? null : options.retentionDays,
      maxSessions: options.maxSessions,
      echoCancellation: options.echoCancellation,
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

export async function saveMeetingReview(request: SaveMeetingReviewRequest): Promise<MeetingDetail> {
  return invoke('save_meeting_review', { request });
}

export async function restoreMeetingReviewFromGenerated(
  sessionId: string,
  generatedRevision: number,
  expectedReviewRevision: number | null,
): Promise<MeetingDetail> {
  return invoke('restore_meeting_review_from_generated', {
    request: { sessionId, generatedRevision, expectedReviewRevision },
  });
}

export async function getMeetingSummaryStatus(): Promise<MeetingSummaryStatus> {
  return invoke('get_meeting_summary_status');
}

export async function startMeetingSummary(sessionId: string): Promise<MeetingSummaryStatus> {
  return invoke('start_meeting_summary', { sessionId });
}

export async function cancelMeetingSummary(): Promise<boolean> {
  return invoke('cancel_meeting_summary');
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

export async function requestSystemAudioPermission(): Promise<SystemAudioAccess> {
  return invoke('request_system_audio_permission');
}

export async function openSystemAudioPreferences(): Promise<void> {
  await invoke('open_system_audio_preferences');
}

export async function copyMeeting(id: string, format: MeetingReviewExportFormat): Promise<void> {
  const text = await invoke<string>('get_meeting_review_export', { id, format });
  await navigator.clipboard.writeText(text);
}

export async function saveMeetingExport(
  id: string,
  startedAtMs: number,
  format: MeetingReviewExportFormat,
): Promise<string | null> {
  const extension = format === 'markdown' ? 'md' : format === 'json' ? 'json' : 'txt';
  const stamp = new Date(startedAtMs).toISOString().slice(0, 10);
  const path = await save({
    defaultPath: `Murmur Meeting ${stamp}.${extension}`,
    filters: [{ name: `Murmur meeting ${format}`, extensions: [extension] }],
  });
  if (typeof path !== 'string' || path.length === 0) return null;
  await invoke('save_meeting_review_export', { id, format, path });
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
