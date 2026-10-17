import { MEETING_SEARCH_FIELD_LABELS } from '../../lib/meetings';
import { Fragment, useEffect, useRef, useState } from 'react';
import { Search, Trash2 } from 'lucide-react';
import { echoCancellationNotice, formatMeetingDuration, meetingErrorMessage, formatMeetingTimestamp } from '../../lib/meetings';
import { historyDayKey, historyDayLabel } from '../../lib/history';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import { MeetingReviewWorkspace } from './MeetingReviewWorkspace';
import { useMeetingAudio } from '../../lib/hooks/useMeetingAudio';
import {
  AnimatedDropdown,
  AnimatedDropdownContent,
  AnimatedDropdownTrigger,
} from '@/components/ui/animated-dropdown/animated-dropdown';
import HoldToDeleteButton from '@/components/ui/hold-to-delete-button/hold-to-delete-button';

interface MeetingsPanelProps {
  meetings: ReturnType<typeof useMeetings>;
  playbackBusy: boolean;
}

const CHANNEL_EXPLAINER = 'Me comes from your microphone; Them comes from Mac playback. If your own voice is played through speakers, it can also appear as Them.';
const SEARCH_DEBOUNCE_MS = 250;

function phaseLabel(phase: ReturnType<typeof useMeetings>['status']['phase']): string {
  if (phase === 'starting') return 'Connecting both channels…';
  if (phase === 'recording') return 'Recording meeting';
  if (phase === 'stopping') return 'Stopping capture…';
  if (phase === 'processing') return 'Finishing pending transcript chunks…';
  if (phase === 'failed') return 'Meeting stopped';
  return 'Ready for a meeting';
}

const meetingTime = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

export function MeetingsPanel({ meetings, playbackBusy }: MeetingsPanelProps) {
  const [query, setQuery] = useState(meetings.appliedQuery);
  const [permissionBusy, setPermissionBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [listMenuOpen, setListMenuOpen] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
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
  const failure = meetingErrorMessage(meetings.status.errorCode);
  const echoCancellationStatus = echoCancellationNotice(meetings.status.echoCancellation);
  const sessions = meetings.page.sessions;
  const { refresh, appliedQuery } = meetings;

  useEffect(() => () => {
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

  const deleteSelected = async () => {
    const id = meetings.detail?.session.id;
    if (!id) return;
    if (confirmDelete !== id) {
      setConfirmDelete(id);
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setConfirmDelete(null), 4000);
      return;
    }
    setConfirmDelete(null);
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

  return (
    <div className="meetings-panel flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="meetings-statusbar">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span aria-hidden="true" className={`h-2 w-2 shrink-0 rounded-full ${
              meetings.status.phase === 'recording' ? 'animate-pulse bg-error' : processing ? 'animate-pulse bg-warning' : 'bg-success'
            }`} />
            <p className="truncate text-sm font-semibold text-on-surface">{phaseLabel(meetings.status.phase)}</p>
            {meetings.status.elapsedMs > 0 && (
              <span className="font-mono text-xs tabular-nums text-on-surface-variant">
                {formatMeetingTimestamp(meetings.status.elapsedMs)}
              </span>
            )}
          </div>
          <p className="meetings-statusbar-detail" title={CHANNEL_EXPLAINER}>
            <span className={meetings.status.microphoneActive ? 'text-primary' : ''}>
              <span aria-hidden="true">● </span>Microphone · Me
            </span>
            <span className={meetings.status.systemAudioActive ? 'text-success' : ''}>
              <span aria-hidden="true">● </span>System Audio · Them
            </span>
            <span className="ui-visually-hidden">{CHANNEL_EXPLAINER}</span>
            {showPermissionHint && (
              <span className="meetings-permission-hint">
                <span>System Audio access is requested only when you explicitly check it or start a meeting.</span>
                <button
                  type="button"
                  disabled={permissionBusy}
                  onClick={() => void requestPermission()}
                  className="font-semibold text-primary hover:underline disabled:opacity-50"
                >
                  {permissionBusy ? 'Waiting…' : 'Check Access'}
                </button>
              </span>
            )}
          </p>
        </div>
        <button
          type="button"
          disabled={processing || meetings.status.phase === 'stopping'}
          onClick={() => void (active ? meetings.stop() : meetings.start())}
          className="ui-dashboard-action shrink-0"
          data-action="primary"
          data-tone={active ? 'danger' : 'default'}
        >
          {active ? 'Stop Meeting' : processing ? 'Finishing…' : 'Start Meeting'}
        </button>
      </div>
      {(failure || meetings.error || echoCancellationStatus || notice) && (
        <div className="meetings-alerts">
          {(failure || meetings.error) && (
            <div role="alert" className="dialog-toast flex items-center gap-2 border-error/25 bg-error/10 px-3 py-2 text-xs text-error">
              <span className="min-w-0 flex-1">{failure ?? meetings.error}</span>
              {meetings.permission === 'denied' && (
                <button type="button" onClick={() => void meetings.openSystemAudioPreferences()} className="shrink-0 font-semibold underline">
                  Open Settings
                </button>
              )}
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

      <div className="meetings-split">
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
                    className="h-9 w-full min-w-0 justify-start rounded-lg px-2.5 text-[length:var(--ui-font-label)]"
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
              const matches = meetings.page.searchMatches[session.id] ?? [];
              return (
                <Fragment key={session.id}>
                  {showDay && <p className="meetings-day-label">{historyDayLabel(session.startedAtMs)}</p>}
                  <button
                    type="button"
                    onClick={() => void meetings.select(session.id)}
                    aria-pressed={selected}
                    className="meetings-row"
                  >
                    <span className="meetings-row-title" data-untitled={!session.title}>
                      <span className="meetings-row-title-text">{session.title || 'Untitled meeting'}</span>
                      {session.status !== 'complete' && (
                        <span className="meetings-row-status" data-status={session.status}>{session.status}</span>
                      )}
                    </span>
                    <span className="meetings-row-time">{meetingTime(session.startedAtMs)}</span>
                    <span className="meetings-row-preview">
                      {session.preview || `${session.segmentCount} transcript ${session.segmentCount === 1 ? 'segment' : 'segments'}`}
                    </span>
                    <span className="meetings-row-duration">{formatMeetingDuration(session.durationMs)}</span>
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
                className="meeting-header-action my-2 w-full justify-center"
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
              headerMeta={[
                new Date(detail.session.startedAtMs).toLocaleString([], { weekday: 'short', month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit' }),
                formatMeetingDuration(detail.session.durationMs),
                detail.session.retainAudio ? 'Audio kept' : 'Transcript only',
              ].join(' · ')}
              headerActions={(
                <button
                  type="button"
                  disabled={active && meetings.status.sessionId === detail.session.id}
                  onClick={() => void deleteSelected()}
                  className="meeting-header-action"
                  data-tone="danger"
                  data-confirming={confirmDelete === detail.session.id}
                  aria-label={confirmDelete === detail.session.id ? 'Confirm Delete' : 'Delete meeting'}
                  title={confirmDelete === detail.session.id ? undefined : 'Delete meeting'}
                >
                  <Trash2 aria-hidden="true" className="h-3.5 w-3.5" />
                  {confirmDelete === detail.session.id ? 'Confirm Delete' : <span className="ui-visually-hidden">Delete</span>}
                </button>
              )}
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
      </div>
    </div>
  );
}
