import { invoke } from '@tauri-apps/api/core';
import type { MeetingDetail } from './meetings';

export type CalendarPermissionState = 'notDetermined' | 'granted' | 'denied' | 'restricted' | 'unsupported';

export interface CalendarEventCandidate {
  selectionToken: string;
  title: string;
  attendees: string[];
  startMs: number;
  endMs: number;
}

export function getCalendarPermissionStatus(): Promise<CalendarPermissionState> {
  return invoke('get_calendar_permission_status');
}

export function requestCalendarPermission(): Promise<CalendarPermissionState> {
  return invoke('request_calendar_permission');
}

export function resetCalendarPermission(): Promise<void> {
  return invoke('reset_calendar_permission');
}

export function openCalendarPreferences(): Promise<void> {
  return invoke('open_calendar_preferences');
}

export function getMeetingCalendarEvents(sessionId: string): Promise<CalendarEventCandidate[]> {
  return invoke('get_meeting_calendar_events', { sessionId });
}

export function applyMeetingCalendarEvent(sessionId: string, selectionToken: string): Promise<MeetingDetail> {
  return invoke('apply_meeting_calendar_event', { sessionId, selectionToken });
}
