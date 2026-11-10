import { MEETING_SEARCH_FIELD_LABELS } from '../../lib/meetings';
import { Fragment, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Search } from 'lucide-react';
import { echoCancellationNotice, formatMeetingDuration, meetingErrorMessage, formatMeetingTimestamp } from '../../lib/meetings';
import { historyDayKey, historyDayLabel } from '../../lib/history';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import { meetingCapturePresentation } from './meetingCapturePresentation';
import { MeetingReviewWorkspace } from './MeetingReviewWorkspace';
import { useMeetingAudio } from '../../lib/hooks/useMeetingAudio';
import {
  AnimatedDropdown,
  AnimatedDropdownContent,
  AnimatedDropdownTrigger,
} from '@/components/ui/animated-dropdown/animated-dropdown';
import { Button } from '../ui/FormControls';
import HoldToDeleteButton from '@/components/ui/hold-to-delete-button/hold-to-delete-button';

interface MeetingsPanelProps {
  meetings: ReturnType<typeof useMeetings>;
  playbackBusy: boolean;
}

const CHANNEL_EXPLAINER = 'Me comes from your microphone; Them comes from Mac playback. If your own voice is played through speakers, it can also appear as Them.';
const SEARCH_DEBOUNCE_MS = 250;

function phaseLabel(phase: ReturnType<typeof useMeetings>['status']['phase']): string {
  if (phase === 'starting') return 'Connecting both channels…';
  if (phase === 'recording') return 'Recording';
  if (phase === 'stopping') return 'Stopping capture…';
  if (phase === 'processing') return 'Finishing pending transcript chunks…';
  if (phase === 'failed') return 'Meeting stopped';
  return 'Meeting capture is idle';
}

const meetingTime = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

export function MeetingsPanel({ meetings, playbackBusy }: MeetingsPanelProps) {
  const [query, setQuery] = useState(meetings.appliedQuery);
  const statusRef = useRef<HTMLDivElement>(null);
  const phaseRef = useRef<HTMLDivElement>(null);
  const sourcesRef = useRef<HTMLDivElement>(null);
  const [statusWrapped, setStatusWrapped] = useState(false);
  const [permissionBusy, setPermissionBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [deleteArmedAt, setDeleteArmedAt] = useState<number | null>(null);
  const armedDeleteSessionRef = useRef<string | null>(null);
  const [listMenuOpen, setListMenuOpen] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const deleteTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const searchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const active = ['starting', 'recording', 'stopping'].includes(meetings.status.phase);
  const processing = meetings.status.phase === 'processing';
  const captureBusy = active || processing || playbackBusy;
  const meetingAudio = useMeetingAudio({
    sessionId: meetings.detail?.session.retainAudio === true
      ? meetings.detail.session.id
      : null,
    captureBusy,
  });
  const visibleSegments = meetings.detail?.segments ?? (
    meetings.status.sessionId ? meetings.liveSegments.filter((segment) => segment.sessionId === meetings.status.sessionId) : []
  );
  const failure = meetings.permission === 'denied' && meetings.status.errorCode === 'system_audio_permission_denied' ? null : meetingErrorMessage(meetings.status.errorCode);
  const echoCancellationStatus = echoCancellationNotice(meetings.status.echoCancellation);
  const sessions = meetings.page.sessions;
  const { refresh, appliedQuery } = meetings;

  useEffect(() => () => {
    if (deleteTimerRef.current) clearTimeout(deleteTimerRef.current);
    if (timerRef.current) clearTimeout(timerRef.current);
  }, []);

  // Search as you type; Enter still searches immediately.
  useEffect(() => {
    if (query.trim() === appliedQuery) return;
    searchTimerRef.current = setTimeout(() => void refresh(query), SEARCH_DEBOUNCE_MS);
    return () => {
      if (searchTimerRef.current) clearTimeout(searchTimerRef.current);
    };
  }, [query, appliedQuery, refresh]);

  const showNotice = (message: string) => {
    setNotice(message);
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setNotice(null), 3500);
  };

  const requestPermission = async () => {
    setPermissionBusy(true);
    try {
      const status = await meetings.requestPermission();
      if (status === 'granted') showNotice('System Audio access is ready.');
    } finally {
      setPermissionBusy(false);
    }
  };

  // Cancellation and activation can arrive in one React batch. The ref is
  // authoritative immediately; state only renders the confirmation label.
  const setDeleteConfirmation = useCallback((sessionId: string | null) => {
    armedDeleteSessionRef.current = sessionId;
    setConfirmDelete(sessionId);
    setDeleteArmedAt(sessionId ? performance.now() : null);
  }, []);

  const disarmDelete = useCallback(() => {
    setDeleteConfirmation(null);
    if (deleteTimerRef.current) clearTimeout(deleteTimerRef.current);
    deleteTimerRef.current = null;
  }, [setDeleteConfirmation]);

  useEffect(() => {
    disarmDelete();
  }, [meetings.detail?.session.id, captureBusy, disarmDelete]);

  const deleteSelected = async () => {
    const id = meetings.detail?.session.id;
    if (!id) return;
    if (armedDeleteSessionRef.current !== id) {
      setDeleteConfirmation(id);
      if (deleteTimerRef.current) clearTimeout(deleteTimerRef.current);
      deleteTimerRef.current = setTimeout(disarmDelete, 4000);
      return;
    }
    disarmDelete();
    meetingAudio.invalidate();
    if (await meetings.remove(id)) {
      showNotice('Meeting deleted from this Mac.');
    }
  };

  const clearAll = async () => {
    setListMenuOpen(false);
    meetingAudio.invalidate();
    if (await meetings.clear()) {
      showNotice('All meeting transcripts deleted from this Mac.');
    }
  };

  const showPermissionHint = meetings.permission !== 'granted' && meetings.permission !== 'unsupported' && !active && !processing;
  const detail = meetings.detail;
  const detailCapture = detail ? meetingCapturePresentation(detail.session.id, meetings.status) : null;

  const denied = meetings.permission === 'denied' && !active && !processing;
  const empty = !meetings.loading && sessions.length === 0 && !appliedQuery && !detail && !active && !processing;
  useLayoutEffect(() => {
    const status = statusRef.current;
    const phase = phaseRef.current;
    const sources = sourcesRef.current;
    if (!status || !phase || !sources) return;
    const syncWrapped = () => setStatusWrapped(sources.getBoundingClientRect().top >= phase.getBoundingClientRect().bottom);
    syncWrapped();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(syncWrapped);
    observer.observe(status);
    observer.observe(phase);
    observer.observe(sources);
    return () => observer.disconnect();
  }, [denied]);

  const startAction = denied ? (
    <Button size="sm" onClick={() => void meetings.start()} className="meetings-start">Start meeting</Button>
  ) : (
    <button type="button" disabled={processing || meetings.status.phase === 'stopping'}
      onClick={() => void (active ? meetings.stop() : meetings.start())}
      className="ui-dashboard-action meetings-start" data-action="primary" data-tone={active ? 'danger' : 'default'}>
      {meetings.status.phase === 'stopping' ? 'Stopping…' : active ? 'Stop meeting' : processing ? 'Finishing…' : 'Start meeting'}
    </button>
  );

  return (
    <div className="meetings-panel flex min-h-0 flex-1 flex-col overflow-hidden">
      <header className="meetings-page-header">
        <h1 id="meetings-view-title">Notetaker</h1>
        {!empty && startAction}
        {denied && <button type="button" className="ui-dashboard-action" data-action="primary" onClick={() => void meetings.openSystemAudioPreferences()}>Open System Settings…</button>}
      </header>
      <div ref={statusRef} className="meetings-statusbar" data-denied={denied} data-wrapped={statusWrapped}>
        <div ref={phaseRef} className="meetings-phase">
          <span aria-hidden="true" className="meetings-phase-dot" data-phase={denied ? 'denied' : meetings.status.phase} />
          <p>{denied ? "Screen & System Audio Recording is off. Murmur can't capture what other people say until you allow it." : phaseLabel(meetings.status.phase)}</p>
          {!denied && meetings.status.elapsedMs > 0 && <span className="meetings-elapsed">{formatMeetingTimestamp(meetings.status.elapsedMs)}</span>}
        </div>
        {!denied && <div ref={sourcesRef} className="meetings-statusbar-detail" title={CHANNEL_EXPLAINER}>
          <span className="meetings-source" data-channel="me" data-active={meetings.status.microphoneActive}><span aria-hidden="true" className="meetings-source-dot" />Microphone (Me)</span>
          <span className="meetings-source" data-channel="them" data-active={meetings.status.systemAudioActive}><span aria-hidden="true" className="meetings-source-dot" />System audio (Them)</span>
          <span className="ui-visually-hidden">{CHANNEL_EXPLAINER}</span>
          {showPermissionHint && <span className="meetings-permission-hint">
            <span><span aria-hidden="true" className="meetings-source-dot" />Mac audio access not checked</span>
            <Button size="xs" disabled={permissionBusy} onClick={() => void requestPermission()} className="meetings-check-access">{permissionBusy ? 'Waiting…' : 'Check access'}</Button>
          </span>}
        </div>}
      </div>
      {(failure || meetings.error || echoCancellationStatus || notice) && (
        <div className="meetings-alerts">
          {(failure || meetings.error) && (
            <div role="alert" className="dialog-toast flex items-center gap-2 border-error/25 bg-error/10 px-3 py-2 text-xs text-error">
              <span className="min-w-0 flex-1">{failure ?? meetings.error}</span>
            </div>
          )}
          {echoCancellationStatus && (
            <p role="status" className="rounded-lg border border-warning/25 bg-warning/10 px-3 py-2 text-xs text-on-surface-variant">
              {echoCancellationStatus}
            </p>
          )}
          {notice && <p role="status" className="text-xs text-success">{notice}</p>}
        </div>
      )}

      {empty ? <section className="meetings-first-meeting"><div>
        <h2>Record your first meeting</h2>
        <p>Capture your microphone and Mac audio in a transcript saved on this Mac.</p>
        {startAction}
      </div></section> : <div className="meetings-split">
        <aside className="meetings-list-column" aria-label="Saved meetings">
          <div className="meetings-list-toolbar">
            <label className="meetings-search">
              <Search aria-hidden="true" className="h-3.5 w-3.5 shrink-0 text-on-surface-variant" />
              <input
                type="search"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    if (searchTimerRef.current) clearTimeout(searchTimerRef.current);
                    void refresh(query);
                  }
                  if (event.key === 'Escape' && query) {
                    event.preventDefault();
                    setQuery('');
                  }
                }}
                placeholder="Search meetings"
                aria-label="Search meetings"
              />
            </label>
            {sessions.length > 0 && !active && !processing && (
              <AnimatedDropdown open={listMenuOpen} onOpenChange={setListMenuOpen}>
                <AnimatedDropdownTrigger className="history-export-trigger" aria-label="More meeting actions">
                  <span aria-hidden="true" className="text-base leading-none">···</span>
                </AnimatedDropdownTrigger>
                <AnimatedDropdownContent align="end" side="bottom" className="history-export-menu min-w-56">
                  <HoldToDeleteButton
                    label="Hold to delete all meetings"
                    confirmLabel="Click again to delete all meetings"
                    holdDuration={1200}
                    onDelete={() => void clearAll()}
                    className="h-auto min-h-(--ui-menu-row) w-full min-w-0 justify-start gap-2 rounded-(--ui-radius-menu-item) px-2 py-1 text-sm shadow-none"
                  />
                </AnimatedDropdownContent>
              </AnimatedDropdown>
            )}
          </div>
          <div className="meetings-list">
            {meetings.loading && sessions.length === 0 && <p className="p-3 text-xs text-on-surface-variant">Loading meetings…</p>}
            {!meetings.loading && sessions.length === 0 && (
              <p className="p-3 text-xs leading-relaxed text-on-surface-variant">
                {appliedQuery
                  ? 'No meetings match this search. Try another word or clear the search.'
                  : 'No meeting transcripts yet. Start one to capture microphone and Mac playback as separate speakers.'}
              </p>
            )}
            {sessions.map((session, index) => {
              const showDay = index === 0 || historyDayKey(session.startedAtMs) !== historyDayKey(sessions[index - 1].startedAtMs);
              const selected = detail?.session.id === session.id;
              const capture = meetingCapturePresentation(session.id, meetings.status);
              const matches = meetings.page.searchMatches[session.id] ?? [];
              return (
                <Fragment key={session.id}>
                  {showDay && <p className="meetings-day-label">{historyDayLabel(session.startedAtMs)}</p>}
                  <button
                    type="button"
                    onClick={() => void meetings.select(session.id)}
                    aria-pressed={selected}
                    className="meetings-row" data-live={capture !== null}
                  >
                    <span className="meetings-row-title" data-untitled={!session.title}>
                      <span className="meetings-row-title-text" title={session.title || `Meeting · ${meetingTime(session.startedAtMs)}`}>{session.title || `Meeting · ${meetingTime(session.startedAtMs)}`}</span>
                      {session.status !== 'complete' && session.status !== 'active' && (
                        <span className="meetings-row-status" data-status={session.status}>{session.status}</span>
                      )}
                    </span>
                    {session.title && <span className="meetings-row-time">{meetingTime(session.startedAtMs)}</span>}
                    <span className="meetings-row-preview">
                      {capture ? <>{capture.phase === 'recording' && <span className="meetings-live-dot" aria-hidden="true" />}{capture.summary}</> : session.preview || `${session.segmentCount} transcript ${session.segmentCount === 1 ? 'segment' : 'segments'}`}
                    </span>
                    {!capture && <span className="meetings-row-duration">{formatMeetingDuration(session.durationMs)}</span>}
                    {matches.length > 0 && (
                      <span className="meetings-row-matches">
                        Matches: {matches.map((field) => MEETING_SEARCH_FIELD_LABELS[field]).join(', ')}
                      </span>
                    )}
                  </button>
                </Fragment>
              );
            })}
            {meetings.hasMore && (
              <button
                type="button"
                className="ui-form-button meeting-header-action my-2 w-full justify-center"
                aria-label="Load more meetings"
                disabled={meetings.loading || meetings.loadingMore || query.trim() !== appliedQuery}
                onClick={() => void meetings.loadMore()}
              >
                Load more meetings
              </button>
            )}
            <p role="status" className="ui-visually-hidden">
              {meetings.loadingMore ? 'Loading more meetings…' : `${sessions.length} meetings shown`}
            </p>
          </div>
        </aside>

        <section className="meetings-detail-column" aria-label="Meeting detail">
          {detail ? (
            <MeetingReviewWorkspace
              meetings={meetings}
              segments={visibleSegments}
              captureBusy={captureBusy}
              meetingAudio={meetingAudio}
              onNotice={showNotice}
              headerMeta={detailCapture
                ? detailCapture.summary
                : [detail.session.title ? new Date(detail.session.startedAtMs).toLocaleString([], { weekday: 'short', month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit' }) : null,
                  formatMeetingDuration(detail.session.durationMs), detail.session.retainAudio ? 'Audio kept' : 'Transcript only'].filter(Boolean).join(' · ')}
              onDelete={() => void deleteSelected()}
              onDisarmDelete={disarmDelete}
              confirmingDelete={confirmDelete === detail.session.id}
              deleteArmedAt={confirmDelete === detail.session.id ? deleteArmedAt : null}
            />
          ) : (
            <div className="grid h-full place-items-center p-8 text-center">
              <div>
                <p className="text-sm font-semibold text-on-surface">Select a meeting</p>
                <p className="mt-1 max-w-xs text-xs leading-relaxed text-on-surface-variant">
                  Durable Me/Them transcripts are searchable and stay local to this Mac.
                </p>
              </div>
            </div>
          )}
        </section>
      </div>}
    </div>
  );
}
