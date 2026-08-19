import { useEffect, useRef, useState } from 'react';
import { meetingErrorMessage, formatMeetingTimestamp, type MeetingSegment } from '../../lib/meetings';
import type { useMeetings } from '../../lib/hooks/useMeetings';

interface MeetingsPanelProps {
  meetings: ReturnType<typeof useMeetings>;
}

function SpeakerRow({ segment }: { segment: MeetingSegment }) {
  return (
    <div className="grid grid-cols-[3.25rem_3.5rem_minmax(0,1fr)] gap-2 border-b border-outline-variant/10 py-2 text-sm last:border-0">
      <span className="font-mono text-[11px] tabular-nums text-on-surface-variant">
        {formatMeetingTimestamp(segment.startMs)}
      </span>
      <span className={`text-xs font-bold ${segment.speaker === 'me' ? 'text-primary' : 'text-success'}`}>
        {segment.speaker === 'me' ? 'Me' : 'Them'}
      </span>
      <p className="min-w-0 whitespace-pre-wrap break-words text-on-surface">{segment.text}</p>
    </div>
  );
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
  const summaryForSelected = meetings.summaryStatus.sessionId === meetings.detail?.session.id
    ? meetings.summaryStatus : null;
  const summaryBusy = summaryForSelected?.phase === 'running' || summaryForSelected?.phase === 'cancelling';

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

  const copySelected = async () => {
    if (!meetings.detail) return;
    if (await meetings.copy(meetings.detail.session.id)) {
      showNotice('Meeting transcript copied.');
    }
  };

  const exportSelected = async () => {
    if (!meetings.detail) return;
    const path = await meetings.exportText(
      meetings.detail.session.id,
      meetings.detail.session.startedAtMs,
    );
    if (path) showNotice('Meeting transcript exported.');
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
      <div className="shrink-0 border-b border-outline-variant/20 px-4 py-3">
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
            className={`rounded-full px-4 py-2 text-xs font-bold transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
              active
                ? 'border border-error/40 bg-error/10 text-error hover:bg-error/15'
                : 'bg-primary text-on-primary hover:brightness-105'
            }`}
          >
            {active ? 'Stop Meeting' : processing ? 'Finishing…' : 'Start Meeting'}
          </button>
        </div>
        <p className="mt-2 text-[11px] leading-relaxed text-on-surface-variant">
          Me comes from your microphone; Them comes from Mac playback. If your own voice is played through speakers, it can also appear as Them.
        </p>
        {(failure || meetings.error) && (
          <div role="alert" className="mt-2 flex items-center gap-2 rounded-lg border border-error/25 bg-error/10 px-3 py-2 text-xs text-error">
            <span className="min-w-0 flex-1">{failure ?? meetings.error}</span>
            {meetings.permission === 'denied' && (
              <button type="button" onClick={() => void meetings.openSystemAudioPreferences()} className="shrink-0 font-semibold underline">
                Open Settings
              </button>
            )}
          </div>
        )}
        {meetings.permission !== 'granted' && meetings.permission !== 'unsupported' && !active && !processing && (
          <div className="mt-2 flex items-center gap-2 rounded-lg bg-surface-container-low px-3 py-2 text-xs text-on-surface-variant">
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
        <aside className="flex min-h-0 flex-col border-r border-outline-variant/20">
          <div className="flex shrink-0 gap-2 border-b border-outline-variant/15 p-2.5">
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') void meetings.refresh(query);
              }}
              placeholder="Search meetings"
              aria-label="Search meetings"
              className="h-8 min-w-0 flex-1 rounded-lg border border-on-surface-variant bg-surface-container-low px-2.5 text-xs text-on-surface outline-none focus:border-primary"
            />
            <button type="button" onClick={() => void meetings.refresh(query)} className="rounded-lg px-2 text-xs font-semibold text-primary hover:bg-surface-container-high">
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
                className={`mb-1 w-full rounded-xl px-3 py-2.5 text-left transition-colors ${
                  meetings.detail?.session.id === session.id
                    ? 'bg-surface-container-high ring-1 ring-inset ring-primary'
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
              className={`m-2 mt-0 rounded-lg border px-3 py-2 text-xs font-semibold ${
                confirmClear ? 'border-error/30 bg-error/10 text-error' : 'border-outline-variant/25 text-on-surface-variant hover:text-error'
              }`}
            >
              {confirmClear ? 'Confirm Delete All' : 'Delete All Meetings'}
            </button>
          )}
        </aside>

        <section className="flex min-h-0 flex-col overflow-hidden">
          {meetings.detail ? (
            <>
              <div className="flex shrink-0 items-center gap-2 border-b border-outline-variant/15 px-4 py-2.5">
                <div className="min-w-0 flex-1">
                  <p className="text-xs font-semibold text-on-surface">
                    {new Date(meetings.detail.session.startedAtMs).toLocaleString()}
                  </p>
                  <p className="mt-0.5 text-[10px] text-on-surface-variant">
                    {formatMeetingTimestamp(meetings.detail.session.durationMs)} · {meetings.detail.session.segmentCount} segments · {meetings.detail.session.retainAudio ? 'audio retained' : 'transcript only'}
                  </p>
                </div>
                <button type="button" onClick={() => void copySelected()} className="rounded-lg px-2 py-1.5 text-xs font-semibold text-on-surface-variant hover:bg-surface-container-low hover:text-primary">Copy</button>
                <button type="button" onClick={() => void exportSelected()} className="rounded-lg px-2 py-1.5 text-xs font-semibold text-on-surface-variant hover:bg-surface-container-low hover:text-primary">Export</button>
                <button
                  type="button"
                  disabled={active || processing || summaryBusy}
                  onClick={() => void meetings.summarize(meetings.detail!.session.id)}
                  className="rounded-lg bg-surface-container-high px-2 py-1.5 text-xs font-semibold text-primary disabled:opacity-40"
                >
                  {meetings.detail.artifact ? 'Retry Summary' : summaryForSelected?.phase === 'failed' || summaryForSelected?.phase === 'cancelled' ? 'Retry Summary' : 'Summarize'}
                </button>
                {summaryBusy && (
                  <button type="button" onClick={() => void meetings.cancelSummary()} className="rounded-lg px-2 py-1.5 text-xs font-semibold text-error">
                    {summaryForSelected?.phase === 'cancelling' ? 'Cancelling…' : 'Cancel Summary'}
                  </button>
                )}
                <button
                  type="button"
                  disabled={active && meetings.status.sessionId === meetings.detail.session.id}
                  onClick={() => void deleteSelected()}
                  className={`rounded-lg px-2 py-1.5 text-xs font-semibold disabled:opacity-40 ${confirmDelete === meetings.detail.session.id ? 'bg-error/10 text-error' : 'text-on-surface-variant hover:text-error'}`}
                >
                  {confirmDelete === meetings.detail.session.id ? 'Confirm Delete' : 'Delete'}
                </button>
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto px-4 py-2">
                {summaryForSelected && summaryForSelected.phase !== 'idle' && (
                  <div role="status" className="mb-3 rounded-xl border border-outline-variant/20 bg-surface-container-low p-3 text-xs text-on-surface-variant">
                    {summaryBusy
                      ? `Summarizing chunk ${summaryForSelected.completedChunks} of ${summaryForSelected.totalChunks || '…'} · ${formatMeetingTimestamp(summaryForSelected.elapsedMs)}`
                      : summaryForSelected.phase === 'failed'
                        ? `Summary failed (${summaryForSelected.errorCode ?? 'generation_failed'}). Retry when ready.`
                        : summaryForSelected.phase === 'cancelled'
                          ? 'Summary cancelled. The prior stored result, if any, was kept.'
                          : summaryForSelected.phase === 'complete'
                            ? `Summary complete in ${formatMeetingTimestamp(summaryForSelected.elapsedMs)} · peak helper RSS ${summaryForSelected.peakRssMb} MB`
                            : 'Summary is stopping…'}
                  </div>
                )}
                {meetings.detail.artifact && (
                  <article className="mb-4 rounded-xl border border-primary/20 bg-surface-container-low p-4">
                    <h3 className="text-sm font-semibold text-on-surface">Meeting summary</h3>
                    <p className="mt-2 text-xs leading-relaxed text-on-surface">{meetings.detail.artifact.summary.text}</p>
                    <p className="mt-1 text-[10px] text-on-surface-variant">Segments {meetings.detail.artifact.summary.sourceSegmentIds.join(', ')}</p>
                    {meetings.detail.artifact.decisions.length > 0 && <h4 className="mt-3 text-xs font-semibold text-on-surface">Decisions</h4>}
                    {meetings.detail.artifact.decisions.map((item) => <p key={`${item.text}-${item.sourceSegmentIds.join()}`} className="mt-1 text-xs text-on-surface">• {item.text} <span className="text-[10px] text-on-surface-variant">[{item.sourceSegmentIds.join(', ')}]</span></p>)}
                    {meetings.detail.artifact.actionItems.length > 0 && <h4 className="mt-3 text-xs font-semibold text-on-surface">Action items</h4>}
                    {meetings.detail.artifact.actionItems.map((item) => <p key={`${item.text}-${item.sourceSegmentIds.join()}`} className="mt-1 text-xs text-on-surface">• {item.text} — {item.owner ?? 'Unknown'} · {item.dueDate ?? 'Unknown'} <span className="text-[10px] text-on-surface-variant">[{item.sourceSegmentIds.join(', ')}]</span></p>)}
                    {meetings.detail.artifact.openQuestions.length > 0 && <h4 className="mt-3 text-xs font-semibold text-on-surface">Open questions</h4>}
                    {meetings.detail.artifact.openQuestions.map((item) => <p key={`${item.text}-${item.sourceSegmentIds.join()}`} className="mt-1 text-xs text-on-surface">• {item.text} <span className="text-[10px] text-on-surface-variant">[{item.sourceSegmentIds.join(', ')}]</span></p>)}
                  </article>
                )}
                {visibleSegments.length === 0 ? (
                  <p className="py-8 text-center text-xs text-on-surface-variant">
                    {active || processing ? 'Transcript segments appear here as speech is processed.' : 'No speech segments were saved.'}
                  </p>
                ) : visibleSegments.map((segment) => <SpeakerRow key={segment.id} segment={segment} />)}
              </div>
            </>
          ) : (
            <div className="grid h-full place-items-center p-8 text-center">
              <div>
                <div className="mx-auto mb-3 grid h-11 w-11 place-items-center rounded-2xl bg-surface-container-high text-primary">◎</div>
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
