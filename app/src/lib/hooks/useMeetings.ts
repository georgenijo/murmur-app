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
  restoreMeetingReviewFromGenerated,
  saveMeetingExport,
  saveMeetingReview,
  startMeeting,
  startMeetingSummary,
  stopMeeting,
  type MeetingDetail,
  type MeetingPage,
  type MeetingRuntimeStatus,
  type MeetingSummaryStatus,
  type MeetingSegment,
  type MeetingReviewExportFormat,
  type SaveMeetingReviewRequest,
  type SystemAudioAccess,
  type SystemAudioPermissionState,
} from '../meetings';

const EMPTY_PAGE: MeetingPage = { sessions: [], total: 0, offset: 0, limit: 50 };
const MAX_LIVE_SEGMENTS = 200;

export function useMeetings(settings: Settings) {
  const [status, setStatus] = useState<MeetingRuntimeStatus>(IDLE_MEETING_STATUS);
  const [permission, setPermission] = useState<SystemAudioPermissionState>('unknown');
  const [access, setAccess] = useState<SystemAudioAccess | null>(null);
  const [page, setPage] = useState<MeetingPage>(EMPTY_PAGE);
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [liveSegments, setLiveSegments] = useState<MeetingSegment[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [summaryStatus, setSummaryStatus] = useState<MeetingSummaryStatus>(IDLE_MEETING_SUMMARY_STATUS);
  const queryRef = useRef('');
  const selectedIdRef = useRef<string | null>(null);
  const selectionTicketRef = useRef(0);

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
    const ticket = ++selectionTicketRef.current;
    selectedIdRef.current = id;
    if (!id) {
      setDetail(null);
      return;
    }
    try {
      const next = await getMeeting(id);
      if (ticket !== selectionTicketRef.current || selectedIdRef.current !== id) return;
      setDetail(next);
      setError(null);
    } catch (cause) {
      if (ticket !== selectionTicketRef.current || selectedIdRef.current !== id) return;
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
      setAccess(next);
      setPermission(next.permission);
      if (next.needsRelaunch) {
        setError('macOS lists Murmur as allowed, but the permission has not reached this session yet. Quit and reopen Murmur.');
      }
      return next.permission;
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

  const copy = useCallback(async (id: string, format: MeetingReviewExportFormat) => {
    setError(null);
    try {
      await copyMeeting(id, format);
      return true;
    } catch (cause) {
      setError(String(cause));
      return false;
    }
  }, []);

  const exportReview = useCallback(async (
    id: string,
    startedAtMs: number,
    format: MeetingReviewExportFormat,
  ) => {
    setError(null);
    try {
      return await saveMeetingExport(id, startedAtMs, format);
    } catch (cause) {
      setError(String(cause));
      return null;
    }
  }, []);

  const saveReview = useCallback(async (request: SaveMeetingReviewRequest) => {
    setError(null);
    try {
      const next = await saveMeetingReview(request);
      if (selectedIdRef.current === request.sessionId) setDetail(next);
      return true;
    } catch (cause) {
      setError(String(cause));
      return false;
    }
  }, []);

  const restoreReview = useCallback(async (
    sessionId: string,
    generatedRevision: number,
    expectedReviewRevision: number | null,
  ) => {
    setError(null);
    try {
      const next = await restoreMeetingReviewFromGenerated(
        sessionId,
        generatedRevision,
        expectedReviewRevision,
      );
      if (selectedIdRef.current === sessionId) setDetail(next);
      return true;
    } catch (cause) {
      setError(String(cause));
      return false;
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
    try {
      if (await cancelMeetingSummary()) {
        setSummaryStatus((current) => ({ ...current, phase: 'cancelling' }));
      }
    } catch (cause) {
      setError(String(cause));
    }
  }, []);

  return {
    status,
    permission,
    access,
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
    exportReview,
    saveReview,
    restoreReview,
    remove,
    clear,
    summarize,
    cancelSummary,
  };
}
