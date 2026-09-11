import { invoke } from '@tauri-apps/api/core';

export interface MeetingSuggestion {
  token: string;
  title: string;
  startMs: number;
  endMs: number;
}

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const MAX_SUGGESTION_TITLE_LENGTH = 200;

export function isMeetingSuggestionToken(value: unknown): value is string {
  return typeof value === 'string' && UUID_PATTERN.test(value);
}

export function isMeetingSuggestion(value: unknown): value is MeetingSuggestion {
  if (typeof value !== 'object' || value === null) return false;
  if (!('token' in value) || !('title' in value) || !('startMs' in value) || !('endMs' in value)) {
    return false;
  }
  return isMeetingSuggestionToken(value.token)
    && typeof value.title === 'string'
    && value.title.length > 0
    && value.title.length <= MAX_SUGGESTION_TITLE_LENGTH * 2
    && Array.from(value.title).length <= MAX_SUGGESTION_TITLE_LENGTH
    && typeof value.startMs === 'number'
    && Number.isSafeInteger(value.startMs)
    && value.startMs >= 0
    && typeof value.endMs === 'number'
    && Number.isSafeInteger(value.endMs)
    && value.endMs > value.startMs;
}

export function isMeetingSuggestionPayload(value: unknown): value is MeetingSuggestion | null {
  return value === null || isMeetingSuggestion(value);
}

export async function configureMeetingSuggestions(enabled: boolean): Promise<void> {
  await invoke('configure_meeting_suggestions', { enabled });
}

export function getMeetingSuggestion(): Promise<MeetingSuggestion | null> {
  return invoke('get_meeting_suggestion');
}

export async function dismissMeetingSuggestion(token: string): Promise<void> {
  await invoke('dismiss_meeting_suggestion', { token });
}
