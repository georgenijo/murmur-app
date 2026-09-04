import { useEffect, useRef, useState } from 'react';
import { meetingErrorMessage, formatMeetingTimestamp } from '../../lib/meetings';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import { MeetingReviewWorkspace } from './MeetingReviewWorkspace';

interface MeetingsPanelProps {
  meetings: ReturnType<typeof useMeetings>;
}

function phaseLabel(phase: ReturnType<typeof useMeetings>['status']['phase']): string {
  if (phase === 'starting') return 'Connecting both channels…';
  if (phase === 'recording') return 'Recording meeting';
  if (phase === 'stopping') return 'Stopping capture…';
  if (phase === 'processing') return 'Finishing pending transcript chunks…';
  if (phase === 'failed') return 'Meeting stopped';
  return 'Ready for a meeting';
}

export function MeetingsPanel({ meetings }: MeetingsPanelProps) {
  const [query, setQuery] = useState('');
  const [permissionBusy, setPermissionBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const active = ['starting', 'recording', 'stopping'].includes(meetings.status.phase);
  const processing = meetings.status.phase === 'processing';
  const visibleSegments = meetings.detail?.segments ?? (
    meetings.status.sessionId ? meetings.liveSegments.filter((segment) => segment.sessionId === meetings.status.sessionId) : []
  );
  const failure = meetingErrorMessage(meetings.status.errorCode);

  useEffect(() => () => {
    if (timerRef.current) clearTimeout(timerRef.current);
  }, []);

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
    if (await meetings.remove(id)) {
      showNotice('Meeting deleted from this Mac.');
    }
  };

  const clearAll = async () => {
    if (!confirmClear) {
      setConfirmClear(true);
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setConfirmClear(false), 4000);
      return;
    }
    setConfirmClear(false);
    if (await meetings.clear()) {
      showNotice('All meeting transcripts deleted from this Mac.');
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="shrink-0 border-b border-[var(--ui-hairline)] px-4 py-3">
        <div className="flex items-center gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className={`h-2 w-2 rounded-full ${
                meetings.status.phase === 'recording' ? 'animate-pulse bg-error' : processing ? 'animate-pulse bg-warning' : 'bg-success'
              }`} />
              <p className="truncate text-sm font-semibold text-on-surface">{phaseLabel(meetings.status.phase)}</p>
              {meetings.status.elapsedMs > 0 && (
                <span className="font-mono text-xs tabular-nums text-on-surface-variant">
                  {formatMeetingTimestamp(meetings.status.elapsedMs)}
                </span>
              )}
            </div>
            <div className="mt-1 flex gap-3 text-[11px] text-on-surface-variant">
              <span className={meetings.status.microphoneActive ? 'text-primary' : ''}>
                ● Microphone · Me
              </span>
              <span className={meetings.status.systemAudioActive ? 'text-success' : ''}>
                ● System Audio · Them
              </span>
            </div>
          </div>
          <button
            type="button"
            disabled={processing || meetings.status.phase === 'stopping'}
            onClick={() => void (active ? meetings.stop() : meetings.start())}
            className={`rounded-[var(--ui-radius-pill)] px-4 py-2 text-xs font-bold transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
              active
                ? 'border border-error/40 bg-error/10 text-error hover:bg-error/15'
                : 'bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))] text-on-primary shadow-[var(--ui-shadow-accent)] hover:brightness-105'
            }`}
          >
            {active ? 'Stop Meeting' : processing ? 'Finishing…' : 'Start Meeting'}
          </button>
        </div>
        <p className="mt-2 text-[11px] leading-relaxed text-on-surface-variant">
          Me comes from your microphone; Them comes from Mac playback. If your own voice is played through speakers, it can also appear as Them.
        </p>
        {(failure || meetings.error) && (
          <div role="alert" className="dialog-toast mt-2 flex items-center gap-2 border-error/25 bg-error/10 px-3 py-2 text-xs text-error">
            <span className="min-w-0 flex-1">{failure ?? meetings.error}</span>
            {meetings.permission === 'denied' && (
              <button type="button" onClick={() => void meetings.openSystemAudioPreferences()} className="shrink-0 font-semibold underline">
                Open Settings
              </button>
            )}
          </div>
        )}
        {meetings.permission !== 'granted' && meetings.permission !== 'unsupported' && !active && !processing && (
          <div className="dialog-card mt-2 flex items-center gap-2 px-3 py-2 text-xs text-on-surface-variant">
            <span className="min-w-0 flex-1">
              System Audio access is requested only when you explicitly check it or start a meeting.
            </span>
            <button
              type="button"
              disabled={permissionBusy}
              onClick={() => void requestPermission()}
              className="shrink-0 font-semibold text-primary underline disabled:opacity-50"
            >
              {permissionBusy ? 'Waiting…' : 'Check Access'}
            </button>
          </div>
        )}
        {notice && <p role="status" className="mt-2 text-xs text-success">{notice}</p>}
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-[minmax(12rem,0.78fr)_minmax(0,1.5fr)] overflow-hidden">
        <aside className="flex min-h-0 flex-col border-r border-[var(--ui-hairline)]">
          <div className="flex shrink-0 gap-2 border-b border-[var(--ui-hairline)] p-2.5">
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') void meetings.refresh(query);
              }}
              placeholder="Search meetings"
              aria-label="Search meetings"
              className="h-8 min-w-0 flex-1 rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-low px-2.5 text-xs text-on-surface outline-none focus:border-primary focus-visible:ring-2 focus-visible:ring-primary"
            />
            <button type="button" onClick={() => void meetings.refresh(query)} className="rounded-[var(--ui-radius-control)] px-2 text-xs font-semibold text-primary hover:bg-surface-container-high">
              Search
            </button>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto p-2">
            {meetings.loading && <p className="p-3 text-xs text-on-surface-variant">Loading meetings…</p>}
            {!meetings.loading && meetings.page.sessions.length === 0 && (
              <p className="p-3 text-xs leading-relaxed text-on-surface-variant">
                No meeting transcripts yet. Start one to capture microphone and Mac playback as separate speakers.
              </p>
            )}
            {meetings.page.sessions.map((session) => (
              <button
                key={session.id}
                type="button"
                onClick={() => void meetings.select(session.id)}
                aria-pressed={meetings.detail?.session.id === session.id}
                className={`mb-1 w-full rounded-[var(--ui-radius-card)] px-3 py-2.5 text-left transition-colors ${
                  meetings.detail?.session.id === session.id
                    ? 'bg-surface-container-high shadow-[var(--ui-shadow-1)] ring-1 ring-inset ring-primary/45'
                    : 'hover:bg-surface-container-low'
                }`}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate text-xs font-semibold text-on-surface">
                    {new Date(session.startedAtMs).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' })}
                  </span>
                  <span className="text-[10px] capitalize text-on-surface-variant">{session.status}</span>
                </div>
                <p className="mt-1 line-clamp-2 text-[11px] leading-relaxed text-on-surface-variant">
                  {session.preview || `${session.segmentCount} transcript ${session.segmentCount === 1 ? 'segment' : 'segments'}`}
                </p>
              </button>
            ))}
          </div>
          {meetings.page.sessions.length > 0 && !active && !processing && (
            <button
              type="button"
              onClick={() => void clearAll()}
              className={`m-2 mt-0 rounded-[var(--ui-radius-control)] border px-3 py-2 text-xs font-semibold ${
                confirmClear ? 'border-error/30 bg-error/10 text-error' : 'border-[var(--ui-hairline)] text-on-surface-variant hover:text-error'
              }`}
            >
              {confirmClear ? 'Confirm Delete All' : 'Delete All Meetings'}
            </button>
          )}
        </aside>

        <section className="flex min-h-0 flex-col overflow-hidden">
          {meetings.detail ? (
            <>
              <div className="flex shrink-0 items-center gap-2 border-b border-[var(--ui-hairline)] px-4 py-2.5">
                <div className="min-w-0 flex-1">
                  <p className="text-xs font-semibold text-on-surface">
                    {new Date(meetings.detail.session.startedAtMs).toLocaleString()}
                  </p>
                  <p className="mt-0.5 text-[10px] text-on-surface-variant">
                    {formatMeetingTimestamp(meetings.detail.session.durationMs)} · {meetings.detail.session.segmentCount} segments · {meetings.detail.session.retainAudio ? 'audio retained' : 'transcript only'}
                  </p>
                </div>
                <button
                  type="button"
                  disabled={active && meetings.status.sessionId === meetings.detail.session.id}
                  onClick={() => void deleteSelected()}
                  className={`rounded-[var(--ui-radius-control)] px-2 py-1.5 text-xs font-semibold disabled:opacity-40 ${confirmDelete === meetings.detail.session.id ? 'bg-error/10 text-error' : 'text-on-surface-variant hover:text-error'}`}
                >
                  {confirmDelete === meetings.detail.session.id ? 'Confirm Delete' : 'Delete'}
                </button>
              </div>
              <MeetingReviewWorkspace
                meetings={meetings}
                segments={visibleSegments}
                captureBusy={active || processing}
                onNotice={showNotice}
              />
            </>
          ) : (
            <div className="grid h-full place-items-center p-8 text-center">
              <div>
                <div className="mx-auto mb-3 grid h-11 w-11 place-items-center rounded-[var(--ui-radius-card)] bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))] text-on-primary shadow-[var(--ui-shadow-accent)]">◎</div>
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
