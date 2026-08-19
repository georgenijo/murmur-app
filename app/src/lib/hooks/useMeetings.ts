import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import type { Settings } from '../settings';
import {
  IDLE_MEETING_STATUS,
  IDLE_MEETING_SUMMARY_STATUS,
  cancelMeetingSummary,
  copyMeeting,
  deleteAllMeetings,
  deleteMeeting,
  getMeeting,
  getMeetingStatus,
  getMeetingSummaryStatus,
  getSystemAudioPermissionStatus,
  listMeetings,
  openSystemAudioPreferences,
  orderedMeetingSegments,
  requestSystemAudioPermission,
  saveMeetingExport,
  startMeeting,
  startMeetingSummary,
  stopMeeting,
  type MeetingDetail,
  type MeetingPage,
  type MeetingRuntimeStatus,
  type MeetingSummaryStatus,
  type MeetingSegment,
  type SystemAudioPermissionState,
} from '../meetings';

const EMPTY_PAGE: MeetingPage = { sessions: [], total: 0, offset: 0, limit: 50 };
const MAX_LIVE_SEGMENTS = 200;

export function useMeetings(settings: Settings) {
  const [status, setStatus] = useState<MeetingRuntimeStatus>(IDLE_MEETING_STATUS);
  const [permission, setPermission] = useState<SystemAudioPermissionState>('unknown');
  const [page, setPage] = useState<MeetingPage>(EMPTY_PAGE);
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [liveSegments, setLiveSegments] = useState<MeetingSegment[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [summaryStatus, setSummaryStatus] = useState<MeetingSummaryStatus>(IDLE_MEETING_SUMMARY_STATUS);
  const queryRef = useRef('');
  const selectedIdRef = useRef<string | null>(null);

  const refresh = useCallback(async (query = queryRef.current) => {
    queryRef.current = query;
    try {
      setPage(await listMeetings(query));
      setError(null);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  const select = useCallback(async (id: string | null) => {
    selectedIdRef.current = id;
    if (!id) {
      setDetail(null);
      return;
    }
    try {
      setDetail(await getMeeting(id));
      setError(null);
    } catch (cause) {
      setError(String(cause));
    }
  }, []);

  useEffect(() => {
    void Promise.all([
      getMeetingStatus().then(setStatus),
      getMeetingSummaryStatus().then(setSummaryStatus),
      getSystemAudioPermissionStatus().then(setPermission),
      refresh(),
    ]).catch((cause) => {
      flog.warn('main', 'Meeting initialization failed', { error: String(cause) });
      setLoading(false);
    });

    let disposed = false;
    const unlisteners: Array<() => void> = [];
    Promise.all([
      listen<MeetingRuntimeStatus>('meeting-status-changed', (event) => {
        setStatus(event.payload);
        if (event.payload.systemAudioActive) setPermission('granted');
        if (event.payload.errorCode === 'system_audio_permission_denied') setPermission('denied');
        if (event.payload.errorCode === 'unsupported_os') setPermission('unsupported');
        if (event.payload.phase === 'idle' || event.payload.phase === 'failed') void refresh();
      }),
      listen<MeetingSegment>('meeting-segment-finalized', (event) => {
        setLiveSegments((current) => orderedMeetingSegments([...current, event.payload], MAX_LIVE_SEGMENTS));
        if (selectedIdRef.current === event.payload.sessionId) {
          setDetail((current) => current && current.session.id === event.payload.sessionId
            ? {
              ...current,
              segments: orderedMeetingSegments(
                [...current.segments, event.payload],
                current.segments.length + 1,
              ),
            }
            : current);
        }
      }),
      listen<{ sessionId: string; segmentId: number; errorCode: string }>(
        'meeting-segment-failed',
        () => setError('A meeting transcript chunk could not be transcribed. Its local audio remains available until you delete the meeting.'),
      ),
      listen<MeetingSummaryStatus>('meeting-summary-status-changed', (event) => {
        setSummaryStatus(event.payload);
        if (event.payload.phase === 'complete' && selectedIdRef.current === event.payload.sessionId) {
          void select(event.payload.sessionId);
        }
      }),
    ]).then((values) => {
      if (disposed) values.forEach((unlisten) => unlisten());
      else unlisteners.push(...values);
    }).catch((cause) => {
      flog.warn('main', 'Meeting event listeners unavailable', { error: String(cause) });
    });
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [refresh]);

  useEffect(() => {
    if (!['starting', 'recording', 'stopping', 'processing'].includes(status.phase)) return;
    const timer = window.setInterval(() => {
      setStatus((current) => (
        ['starting', 'recording', 'stopping', 'processing'].includes(current.phase)
          ? { ...current, elapsedMs: current.elapsedMs + 1_000 }
          : current
      ));
    }, 1_000);
    return () => window.clearInterval(timer);
  }, [status.phase]);

  const start = useCallback(async () => {
    setError(null);
    setLiveSegments([]);
    try {
      const session = await startMeeting({
        microphone: settings.microphone,
        retainAudio: settings.meetingRetainAudio,
        retentionDays: settings.meetingRetentionDays,
        maxSessions: settings.meetingMaxSessions,
      });
      setPage((current) => ({
        ...current,
        sessions: [session, ...current.sessions.filter((item) => item.id !== session.id)],
        total: current.total + 1,
      }));
      await select(session.id);
    } catch (cause) {
      setError(String(cause));
    }
  }, [select, settings.meetingMaxSessions, settings.meetingRetainAudio, settings.meetingRetentionDays, settings.microphone]);

  const stop = useCallback(async () => {
    setError(null);
    try {
      await stopMeeting();
    } catch (cause) {
      setError(String(cause));
    }
  }, []);

  const requestPermission = useCallback(async () => {
    setError(null);
    try {
      const next = await requestSystemAudioPermission();
      setPermission(next);
      return next;
    } catch (cause) {
      setError(String(cause));
      return 'unknown' as const;
    }
  }, []);

  const remove = useCallback(async (id: string) => {
    setError(null);
    try {
      await deleteMeeting(id);
      if (selectedIdRef.current === id) await select(null);
      await refresh();
      return true;
    } catch (cause) {
      setError(String(cause));
      return false;
    }
  }, [refresh, select]);

  const clear = useCallback(async () => {
    setError(null);
    try {
      await deleteAllMeetings();
      await select(null);
      await refresh();
      return true;
    } catch (cause) {
      setError(String(cause));
      return false;
    }
  }, [refresh, select]);

  const copy = useCallback(async (id: string) => {
    setError(null);
    try {
      await copyMeeting(id);
      return true;
    } catch (cause) {
      setError(String(cause));
      return false;
    }
  }, []);

  const exportText = useCallback(async (id: string, startedAtMs: number) => {
    setError(null);
    try {
      return await saveMeetingExport(id, startedAtMs);
    } catch (cause) {
      setError(String(cause));
      return null;
    }
  }, []);

  const openPreferences = useCallback(async () => {
    setError(null);
    try {
      await openSystemAudioPreferences();
    } catch (cause) {
      setError(String(cause));
    }
  }, []);

  const summarize = useCallback(async (sessionId: string) => {
    setError(null);
    try {
      setSummaryStatus(await startMeetingSummary(sessionId));
    } catch (cause) {
      setError(String(cause));
    }
  }, []);

  const cancelSummary = useCallback(async () => {
    if (await cancelMeetingSummary()) {
      setSummaryStatus((current) => ({ ...current, phase: 'cancelling' }));
    }
  }, []);

  return {
    status,
    permission,
    page,
    detail,
    liveSegments,
    loading,
    error,
    summaryStatus,
    refresh,
    select,
    start,
    stop,
    requestPermission,
    openSystemAudioPreferences: openPreferences,
    copy,
    exportText,
    remove,
    clear,
    summarize,
    cancelSummary,
  };
}
