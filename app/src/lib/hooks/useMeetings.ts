import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import { applyMeetingCalendarEvent } from '../calendar';
import { smartAutoMicrophoneRequest, type Settings } from '../settings';
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
  previewMeetingBundle,
  requestSystemAudioPermission,
  renameMeetingRemoteSpeaker,
  restoreMeetingReviewFromGenerated,
  saveMeetingBundle,
  saveMeetingExport,
  saveMeetingReview,
  saveMeetingMetadata,
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
  type SaveMeetingMetadataRequest,
  type SystemAudioAccess,
  type SystemAudioPermissionState,
} from '../meetings';

const EMPTY_PAGE: MeetingPage = { sessions: [], searchMatches: {}, total: 0, offset: 0, limit: 50 };
const MAX_LIVE_SEGMENTS = 200;
const MAX_MEETING_PAGE_SIZE = 100; // Rust's per-request clamp; loaded windows can span requests.

export function useMeetings(settings: Settings) {
  const [status, setStatus] = useState<MeetingRuntimeStatus>(IDLE_MEETING_STATUS);
  const [permission, setPermission] = useState<SystemAudioPermissionState>('unknown');
  const [access, setAccess] = useState<SystemAudioAccess | null>(null);
  const [page, setPage] = useState<MeetingPage>(EMPTY_PAGE);
  const pageRef = useRef<MeetingPage>(EMPTY_PAGE);
  const loadedWindowRef = useRef<{ query: string; pageSize: number; pages: number } | null>(null);
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [liveSegments, setLiveSegments] = useState<MeetingSegment[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const pageCursorRef = useRef<{ offset: number; limit: number } | null>(null);
  const listRequestRef = useRef<'refresh' | 'more' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [summaryStatus, setSummaryStatus] = useState<MeetingSummaryStatus>(IDLE_MEETING_SUMMARY_STATUS);
  const queryRef = useRef('');
  const [appliedQuery, setAppliedQuery] = useState('');
  const searchTicketRef = useRef(0);
  const selectedIdRef = useRef<string | null>(null);
  const selectionTicketRef = useRef(0);
  const detailRequestTicketRef = useRef(0);
  const runtimeStatusRef = useRef<MeetingRuntimeStatus>(IDLE_MEETING_STATUS);
  const terminalReceiptRef = useRef<string | null>(null);

  const refresh = useCallback(async (query = queryRef.current) => {
    const previous = loadedWindowRef.current;
    const previousTotal = pageRef.current.total;
    const sameFilter = previous?.query === query.trim();
    let pageSize = sameFilter ? previous.pageSize : EMPTY_PAGE.limit;
    const pages = sameFilter ? previous.pages : 1;
    let targetRows = pages * pageSize;
    let maxTargetRows = targetRows;
    // Only retain a selection that belonged to this visible filter/window.
    const selected = sameFilter && pageRef.current.sessions.some((session) => session.id === selectedIdRef.current)
      ? selectedIdRef.current : null;
    queryRef.current = query;
    const ticket = ++searchTicketRef.current;
    listRequestRef.current = 'refresh';
    pageCursorRef.current = null;
    setLoadingMore(false);
    setHasMore(false);
    setLoading(true);
    try {
      // Rebuild from offset zero so deleted rows cannot leave holes between an
      // old prefix and a new page. Publish the entire replacement atomically.
      const sessions = new Map<string, MeetingPage['sessions'][number]>();
      let searchMatches: MeetingPage['searchMatches'] = {};
      let offset = 0;
      let total = 0;
      let more = false;
      do {
        const limit = Math.min(MAX_MEETING_PAGE_SIZE, Math.max(pageSize, targetRows - offset));
        const next = offset === 0 && limit === EMPTY_PAGE.limit
          ? await listMeetings(query)
          : await listMeetings(query, offset, limit);
        if (ticket !== searchTicketRef.current) return;
        if (offset === 0 && !sameFilter) {
          pageSize = Math.min(MAX_MEETING_PAGE_SIZE, Math.max(1, next.limit));
          targetRows = pageSize;
        }
        if (offset === 0) {
          // Freeze the selection budget at the first response. A missing or
          // pruned selection must not turn a refresh into a full-list scan.
          const growthPages = sameFilter ? Math.ceil(Math.max(0, next.total - previousTotal) / pageSize) : 0;
          maxTargetRows = targetRows + growthPages * pageSize;
        }
        next.sessions.forEach((session) => sessions.set(session.id, session));
        searchMatches = { ...searchMatches, ...next.searchMatches };
        offset += next.sessions.length;
        total = next.total;
        more = next.sessions.length > 0 && offset < total;
        // New sessions can move the selected row just beyond the old window.
        if (more && offset >= targetRows && targetRows < maxTargetRows
          && selected && selectedIdRef.current === selected && !sessions.has(selected)) {
          targetRows += pageSize;
        }
      } while (more && offset < targetRows);
      const next = { sessions: [...sessions.values()], searchMatches, total, offset: 0, limit: pageSize };
      pageRef.current = next;
      setPage(next);
      loadedWindowRef.current = { query: query.trim(), pageSize, pages: Math.ceil(targetRows / pageSize) };
      pageCursorRef.current = { offset, limit: pageSize };
      setHasMore(more);
      setAppliedQuery(query.trim());
      setError(null);
    } catch (cause) {
      if (ticket === searchTicketRef.current) setError(String(cause));
    } finally {
      if (ticket === searchTicketRef.current) {
        listRequestRef.current = null;
        setLoading(false);
      }
    }
  }, []);

  const loadMore = useCallback(async () => {
    const cursor = pageCursorRef.current;
    if (listRequestRef.current || !cursor || !hasMore) return;
    const ticket = ++searchTicketRef.current;
    listRequestRef.current = 'more';
    setLoadingMore(true);
    try {
      const next = await listMeetings(queryRef.current, cursor.offset, Math.min(MAX_MEETING_PAGE_SIZE, cursor.limit));
      if (ticket !== searchTicketRef.current) return;
      // Advance by fetched rows, not unique rows: insertions can make adjacent
      // offset pages overlap. A refresh replaces the prefix and invalidates it.
      const offset = next.offset + next.sessions.length;
      pageCursorRef.current = { offset, limit: next.limit };
      setHasMore(next.sessions.length > 0 && offset < next.total);
      const current = pageRef.current;
      const sessions = new Map(current.sessions.map((session) => [session.id, session]));
      next.sessions.forEach((session) => sessions.set(session.id, session));
      const merged = {
        ...current, total: next.total, sessions: [...sessions.values()],
        searchMatches: { ...current.searchMatches, ...next.searchMatches },
      };
      pageRef.current = merged;
      setPage(merged);
      const loaded = loadedWindowRef.current;
      if (loaded) loadedWindowRef.current = { ...loaded, pages: loaded.pages + 1 };
      setError(null);
    } catch (cause) {
      if (ticket === searchTicketRef.current) setError(String(cause));
    } finally {
      if (ticket === searchTicketRef.current) {
        listRequestRef.current = null;
        setLoadingMore(false);
      }
    }
  }, [hasMore]);

  const select = useCallback(async (id: string | null) => {
    const ticket = ++selectionTicketRef.current;
    const requestTicket = ++detailRequestTicketRef.current;
    selectedIdRef.current = id;
    if (!id) {
      setDetail(null);
      return;
    }
    try {
      const next = await getMeeting(id);
      if (ticket !== selectionTicketRef.current || requestTicket !== detailRequestTicketRef.current || selectedIdRef.current !== id) return;
      setDetail(next);
      setError(null);
    } catch (cause) {
      if (ticket !== selectionTicketRef.current || requestTicket !== detailRequestTicketRef.current || selectedIdRef.current !== id) return;
      setError(String(cause));
    }
  }, []);

  const refreshFinalizedDetail = useCallback(async (id: string, generation: number) => {
    const ticket = ++detailRequestTicketRef.current;
    const isCurrent = () => ticket === detailRequestTicketRef.current
      && selectedIdRef.current === id && runtimeStatusRef.current.generation === generation;
    try {
      const next = await getMeeting(id);
      if (!isCurrent()) return;
      // Capture completion must not replace review revisions/labels and reset
      // the workspace's unsaved editor, or undo concurrently saved metadata.
      setDetail((current) => current?.session.id === id ? {
        ...current,
        session: {
          ...current.session,
          status: next.session.status,
          endedAtMs: next.session.endedAtMs,
          durationMs: next.session.durationMs,
          segmentCount: next.session.segmentCount,
          preview: next.session.preview,
          errorCode: next.session.errorCode,
        },
        segments: next.segments,
      } : next);
    } catch (cause) {
      if (isCurrent()) setError(String(cause));
    }
  }, []);

  useEffect(() => {
    const initialStatus = runtimeStatusRef.current;
    void Promise.all([
      getMeetingStatus().then((next) => {
        if (runtimeStatusRef.current !== initialStatus) return;
        runtimeStatusRef.current = next;
        setStatus(next);
      }),
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
        const previous = runtimeStatusRef.current;
        if (event.payload.generation < previous.generation) return;
        runtimeStatusRef.current = event.payload;
        setStatus(event.payload);
        if (event.payload.systemAudioActive) setPermission('granted');
        if (event.payload.errorCode === 'system_audio_permission_denied') setPermission('denied');
        if (event.payload.errorCode === 'unsupported_os') setPermission('unsupported');
        if (event.payload.phase === 'idle' || event.payload.phase === 'failed') {
          void refresh();
          // Successful idle drops sessionId; only the same generation's last
          // runtime status identifies the session that actually just finished.
          const id = event.payload.sessionId
            ?? (event.payload.generation === previous.generation ? previous.sessionId : null);
          const receipt = id ? `${event.payload.generation}:${id}` : null;
          if (id && receipt !== terminalReceiptRef.current) {
            terminalReceiptRef.current = receipt;
            if (selectedIdRef.current === id) void refreshFinalizedDetail(id, event.payload.generation);
          }
        }
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
        if (event.payload.phase === 'complete') {
          void refresh();
          if (selectedIdRef.current === event.payload.sessionId) void select(event.payload.sessionId);
        }
      }),
      listen<{ sessionId: string }>('meeting-speakers-updated', (event) => {
        if (selectedIdRef.current === event.payload.sessionId) {
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
      ++searchTicketRef.current;
      ++detailRequestTicketRef.current;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [refresh, refreshFinalizedDetail, select]);

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

  const start = useCallback(async (suggestionToken?: string) => {
    setError(null);
    setLiveSegments([]);
    try {
      const session = await startMeeting({
        microphone: settings.microphone,
        smartAuto: smartAutoMicrophoneRequest(settings),
        retainAudio: settings.meetingRetainAudio,
        retentionDays: settings.meetingRetentionDays,
        maxSessions: settings.meetingMaxSessions,
        echoCancellation: settings.meetingEchoCancellationEnabled,
        diarization: settings.meetingDiarization,
        suggestionToken,
      });
      // Capture can insert/prune rows and move every offset. Rebase the list
      // using its current search instead of prepending into a paginated prefix.
      await refresh();
      await select(session.id);
    } catch (cause) {
      if (suggestionToken) {
        const message = 'Notetaker could not start from that Calendar suggestion. Start it manually to continue.';
        setError(message);
        throw new Error(message);
      }
      setError(String(cause));
    }
  }, [
    select,
    refresh,
    settings.meetingDiarization,
    settings.meetingEchoCancellationEnabled,
    settings.meetingMaxSessions,
    settings.meetingRetainAudio,
    settings.meetingRetentionDays,
    settings.microphone,
    settings.smartAutoAllowContinuity,
    settings.smartAutoApprovedDeviceIds,
    settings.smartAutoMicrophoneEnabled,
    settings.smartAutoProbeEnabled,
    settings.smartAutoPreferredDeviceIds,
  ]);

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

  const previewBundle = useCallback(async (id: string) => {
    setError(null);
    try {
      return await previewMeetingBundle(id);
    } catch (cause) {
      setError(String(cause));
      return null;
    }
  }, []);

  const exportBundle = useCallback(async (
    id: string,
    startedAtMs: number,
    format: MeetingReviewExportFormat,
  ) => {
    setError(null);
    try {
      return await saveMeetingBundle(id, startedAtMs, format);
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
      await refresh();
      return true;
    } catch (cause) {
      setError(String(cause));
      return false;
    }
  }, [refresh]);

  const saveMetadata = useCallback(async (request: SaveMeetingMetadataRequest) => {
    const ticket = selectionTicketRef.current;
    setError(null);
    try {
      const next = await saveMeetingMetadata(request);
      if (ticket === selectionTicketRef.current && selectedIdRef.current === request.sessionId) {
        setDetail(next);
      }
      await refresh();
      return true;
    } catch {
      if (ticket === selectionTicketRef.current && selectedIdRef.current === request.sessionId) {
        setError('Meeting details could not be saved. Use a title and attendee names of at most 200 characters, with up to 100 attendees.');
      }
      return false;
    }
  }, [refresh]);

  const applyCalendarEvent = useCallback(async (sessionId: string, selectionToken: string) => {
    const ticket = selectionTicketRef.current;
    setError(null);
    try {
      const next = await applyMeetingCalendarEvent(sessionId, selectionToken);
      if (ticket === selectionTicketRef.current && selectedIdRef.current === sessionId) setDetail(next);
      await refresh();
      return true;
    } catch {
      if (ticket === selectionTicketRef.current && selectedIdRef.current === sessionId) {
        setError('Calendar details could not be applied. Look up the event again, or name this meeting manually.');
      }
      return false;
    }
  }, [refresh]);

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
      await refresh();
      return true;
    } catch (cause) {
      setError(String(cause));
      return false;
    }
  }, [refresh]);

  const renameRemoteSpeaker = useCallback(async (
    sessionId: string,
    speakerId: number,
    label: string,
  ) => {
    setError(null);
    const ticket = selectionTicketRef.current;
    try {
      const next = await renameMeetingRemoteSpeaker(sessionId, speakerId, label);
      if (ticket === selectionTicketRef.current && selectedIdRef.current === sessionId) {
        setDetail(next);
      }
      return true;
    } catch (cause) {
      if (ticket === selectionTicketRef.current && selectedIdRef.current === sessionId) {
        setError(String(cause));
      }
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
    page, appliedQuery,
    detail,
    liveSegments,
    loading,
    loadingMore,
    hasMore,
    loadMore,
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
    previewBundle,
    exportBundle,
    saveReview,
    saveMetadata,
    applyCalendarEvent,
    restoreReview,
    renameRemoteSpeaker,
    remove,
    clear,
    summarize,
    cancelSummary,
  };
}
