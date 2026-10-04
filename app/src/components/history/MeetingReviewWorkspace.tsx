import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import { MeetingMetadataEditor } from './MeetingMetadataEditor';
import {
  formatMeetingTimestamp,
  MEETING_EXPORT_FORMATS,
  meetingSegmentDisplayLabel,
  type EditableReviewDocument,
  type MeetingReviewDocumentV1,
  type NewReviewClaim,
  type MeetingReviewExportFormat,
  type MeetingSegment,
  type MeetingSpeakerLabels,
  type RemoteSpeakerLabel,
  type ReviewEditBase,
} from '../../lib/meetings';
import type { MeetingAudioController } from '../../lib/hooks/useMeetingAudio';
import { MeetingAudioPlayer } from './MeetingAudioPlayer';

interface MeetingReviewWorkspaceProps {
  meetings: ReturnType<typeof useMeetings>;
  segments: MeetingSegment[];
  captureBusy: boolean;
  meetingAudio?: MeetingAudioController;
  onNotice: (message: string) => void;
}

interface WorkspaceActivation {
  readonly sessionId: string;
}

const editable = (document: MeetingReviewDocumentV1): EditableReviewDocument => ({
  summary: { key: document.summary.key, text: document.summary.text },
  decisions: document.decisions.map(({ key, text }) => ({ key, text })),
  actionItems: document.actionItems.map(({ key, text, owner, dueDate }) => ({ key, text, owner, dueDate })),
  openQuestions: document.openQuestions.map(({ key, text }) => ({ key, text })),
});

function SourceLinks({ label, ids, onActivate }: {
  label: string;
  ids: number[];
  onActivate: (id: number) => void;
}) {
  return (
    <span className="ml-1 inline-flex flex-wrap gap-1">
      {ids.map((id, index) => (
        <button
          key={id}
          type="button"
          aria-controls={`meeting-segment-${id}`}
          aria-label={`${label} source ${index + 1} of ${ids.length}, transcript segment ${id}`}
          onClick={() => onActivate(id)}
          className="rounded-[var(--ui-radius-control)] bg-surface-container-high px-1.5 py-0.5 text-[10px] font-semibold text-primary hover:brightness-95 focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary"
        >
          #{id}
        </button>
      ))}
    </span>
  );
}

function TranscriptRow({ segment, labels, remoteSpeakers, onPlay, playbackDisabled, selected, onSelect, selectionDisabled }: {
  segment: MeetingSegment;
  labels: { me: string; them: string };
  remoteSpeakers: RemoteSpeakerLabel[];
  onPlay?: () => void;
  playbackDisabled?: boolean;
  selected: boolean;
  onSelect: () => void;
  selectionDisabled: boolean;
}) {
  const canonical = segment.speaker === 'me' ? 'Me' : 'Them';
  const display = meetingSegmentDisplayLabel(segment, labels, remoteSpeakers);
  return (
    <article
      id={`meeting-segment-${segment.id}`}
      tabIndex={-1}
      aria-label={`${canonical} channel, ${display}, at ${formatMeetingTimestamp(segment.startMs)}`}
      className="grid scroll-m-20 grid-cols-[3.75rem_7rem_minmax(0,1fr)] gap-2 border-b border-[var(--ui-hairline)] py-2 text-sm last:border-0 focus-visible:rounded-[var(--ui-radius-control)] focus-visible:bg-primary-container/30 focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary"
    >
      {onPlay ? (
        <button
          type="button"
          disabled={playbackDisabled}
          aria-label={`Play segment at ${formatMeetingTimestamp(segment.startMs)}, ${canonical} channel`}
          onClick={onPlay}
          className="w-fit rounded-[var(--ui-radius-control)] font-mono text-[11px] tabular-nums text-primary hover:bg-primary-container/30 focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary disabled:text-on-surface-variant disabled:opacity-50"
        >
          ▶ {formatMeetingTimestamp(segment.startMs)}
        </button>
      ) : (
        <span className="font-mono text-[11px] tabular-nums text-on-surface-variant">{formatMeetingTimestamp(segment.startMs)}</span>
      )}
      <span className={`truncate text-xs font-bold ${segment.speaker === 'me' ? 'text-primary' : 'text-success'}`} title={`${display} (${canonical})`}>
        {display} <span className="font-normal text-on-surface-variant">({canonical})</span>
      </span>
      <div className="min-w-0">
        <label className="mb-1 flex w-fit items-center gap-1.5 text-[11px] text-on-surface-variant">
          <input type="checkbox" checked={selected} onChange={onSelect} disabled={selectionDisabled} aria-label={`Select transcript segment ${segment.id} at ${formatMeetingTimestamp(segment.startMs)}`} aria-controls={`meeting-segment-${segment.id}`} className="accent-primary" />
          Use as source
        </label>
        <p className="min-w-0 whitespace-pre-wrap break-words text-on-surface">
          {segment.status === 'final' ? segment.text : segment.status === 'pending' ? 'Transcript pending…' : 'Transcription failed.'}
        </p>
      </div>
    </article>
  );
}

export function MeetingReviewWorkspace({ meetings, segments, captureBusy, meetingAudio, onNotice }: MeetingReviewWorkspaceProps) {
  const detail = meetings.detail!;
  const activeWorkspace = useRef<WorkspaceActivation | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<EditableReviewDocument | null>(null);
  const [labelDrafts, setLabelDrafts] = useState<Partial<MeetingSpeakerLabels>>({});
  const [remoteSpeakerDrafts, setRemoteSpeakerDrafts] = useState<Partial<Record<number, string>>>({});
  const [format, setFormat] = useState<MeetingReviewExportFormat>('markdown');
  const captions = format === 'srt' || format === 'vtt';
  const [restoreConfirm, setRestoreConfirm] = useState(false);
  const [selectedIds, setSelectedIds] = useState<number[]>([]);
  const [newClaims, setNewClaims] = useState<Array<{ id: number; claim: NewReviewClaim }>>([]);
  const nextClaimId = useRef(0);
  const [saving, setSaving] = useState(false);
  const summaryStatus = meetings.summaryStatus.sessionId === detail.session.id ? meetings.summaryStatus : null;
  const summaryBusy = summaryStatus?.phase === 'running' || summaryStatus?.phase === 'cancelling';
  const activeDocument = detail.activeDocument;
  const labels = {
    me: labelDrafts.me ?? detail.labels.me,
    them: labelDrafts.them ?? detail.labels.them,
  };
  const remoteSpeakers = detail.remoteSpeakers.map((speaker) => ({
    ...speaker,
    label: remoteSpeakerDrafts[speaker.speakerId] ?? speaker.label,
  }));
  const sourceById = useMemo(() => new Map(segments.map((segment) => [segment.id, segment])), [segments]);

  useEffect(() => {
    setEditing(false);
    setDraft(null);
    setRestoreConfirm(false);
    setSelectedIds([]);
    setNewClaims([]);
    setSaving(false);
  }, [detail.session.id, detail.review?.revision, detail.generated?.revision]);

  useEffect(() => {
    setLabelDrafts({});
    setRemoteSpeakerDrafts({});
  }, [detail.session.id]);

  useLayoutEffect(() => {
    const activation: WorkspaceActivation = { sessionId: detail.session.id };
    activeWorkspace.current = activation;
    return () => {
      if (activeWorkspace.current === activation) activeWorkspace.current = null;
    };
  }, [detail.session.id]);

  const isCurrentWorkspace = (activation: WorkspaceActivation | null) => (
    activation !== null && activeWorkspace.current === activation
  );

  const jumpToSource = (id: number) => {
    const target = window.document.getElementById(`meeting-segment-${id}`);
    if (!target || !sourceById.has(id)) {
      onNotice(`Transcript segment ${id} is unavailable.`);
      return;
    }
    target.scrollIntoView({
      block: 'center',
      behavior: window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth',
    });
    target.focus({ preventScroll: true });
    const segment = sourceById.get(id)!;
    onNotice(`Focused source ${id} at ${formatMeetingTimestamp(segment.startMs)}.`);
  };

  const beginEdit = () => {
    if (!activeDocument) return;
    setDraft(editable(activeDocument));
    setEditing(true);
  };

  const saveLabels = async () => {
    const activation = activeWorkspace.current;
    const sessionId = detail.session.id;
    const submitted = labels;
    const saved = await meetings.saveReview({
      sessionId,
      expectedReviewRevision: detail.review?.revision ?? null,
      base: { kind: 'labels_only' },
      labels: submitted,
      document: null,
    });
    if (saved && isCurrentWorkspace(activation)) {
      setLabelDrafts((current) => {
        const next = { ...current };
        if (current.me === submitted.me) delete next.me;
        if (current.them === submitted.them) delete next.them;
        return next;
      });
      onNotice('Speaker labels saved on this Mac.');
    }
  };

  const saveRemoteSpeaker = async (speaker: RemoteSpeakerLabel) => {
    const activation = activeWorkspace.current;
    const sessionId = detail.session.id;
    if (
      await meetings.renameRemoteSpeaker(sessionId, speaker.speakerId, speaker.label)
      && isCurrentWorkspace(activation)
    ) {
      setRemoteSpeakerDrafts((current) => {
        if (current[speaker.speakerId] !== speaker.label) return current;
        const next = { ...current };
        delete next[speaker.speakerId];
        return next;
      });
      onNotice(`${speaker.label.trim()} saved for this meeting.`);
    }
  };

  const createClaim = (kind: NewReviewClaim['kind']) => {
    if (!selectedIds.length) {
      onNotice('Select at least one transcript segment before creating a claim.');
      return;
    }
    if (!activeDocument || saving || captureBusy) return;
    if (!editing) beginEdit();
    const claim: NewReviewClaim = kind === 'action_item'
      ? { kind, text: '', owner: null, dueDate: null, sourceSegmentIds: [...selectedIds] }
      : { kind, text: '', sourceSegmentIds: [...selectedIds] };
    const id = nextClaimId.current++;
    setNewClaims((current) => [...current, { id, claim }]);
    setSelectedIds([]);
    onNotice('New claim added to your unsaved review. Enter its text, then save the review.');
    const activation = activeWorkspace.current;
    window.requestAnimationFrame(() => {
      if (isCurrentWorkspace(activation)) window.document.getElementById(`new-meeting-claim-${id}`)?.focus();
    });
  };

  const saveEdits = async () => {
    if (saving) return;
    if (newClaims.some(({ claim }) => !claim.sourceSegmentIds.length || !claim.text.trim())) {
      onNotice('Every new claim needs text and at least one selected transcript source.');
      return;
    }
    setSaving(true);
    const activation = activeWorkspace.current;
    const sessionId = detail.session.id;
    const base: ReviewEditBase = detail.activeOrigin === 'reviewed' && detail.review
      ? { kind: 'review', reviewRevision: detail.review.revision }
      : detail.generated
        ? { kind: 'generated', generatedRevision: detail.generated.revision }
        : { kind: 'labels_only' };
    const saved = await meetings.saveReview({
      sessionId,
      expectedReviewRevision: detail.review?.revision ?? null,
      base,
      labels,
      document: draft ?? (activeDocument ? editable(activeDocument) : null),
      newClaims: newClaims.map(({ claim }) => claim),
    });
    if (saved && isCurrentWorkspace(activation)) {
      setLabelDrafts((current) => {
        const next = { ...current };
        if (current.me === labels.me) delete next.me;
        if (current.them === labels.them) delete next.them;
        return next;
      });
      setEditing(false);
      setDraft(null);
      setNewClaims([]);
      setSelectedIds([]);
      onNotice('Meeting review saved on this Mac.');
    }
    if (isCurrentWorkspace(activation)) setSaving(false);
  };

  const restore = async () => {
    if (!detail.generated) return;
    if (!restoreConfirm) {
      setRestoreConfirm(true);
      return;
    }
    setRestoreConfirm(false);
    const activation = activeWorkspace.current;
    const sessionId = detail.session.id;
    if (
      await meetings.restoreReview(sessionId, detail.generated.revision, detail.review?.revision ?? null)
      && isCurrentWorkspace(activation)
    ) {
      onNotice('Review replaced with the generated draft. Raw transcript evidence was unchanged.');
    }
  };

  const copy = async () => {
    const activation = activeWorkspace.current;
    const sessionId = detail.session.id;
    if (await meetings.copy(sessionId, format) && isCurrentWorkspace(activation)) {
      onNotice(`Meeting ${captions ? 'captions' : 'review'} copied as ${format}.`);
    }
  };

  const exportReview = async () => {
    const activation = activeWorkspace.current;
    const sessionId = detail.session.id;
    const path = await meetings.exportReview(sessionId, detail.session.startedAtMs, format);
    if (path && isCurrentWorkspace(activation)) onNotice(`Meeting ${captions ? 'captions' : 'review'} exported as ${format}.`);
  };

  const renderTextItems = (title: string, items: MeetingReviewDocumentV1['decisions']) => (
    <section className="mt-3" aria-labelledby={`meeting-${title.toLowerCase().replace(/\s/g, '-')}`}>
      <h4 id={`meeting-${title.toLowerCase().replace(/\s/g, '-')}`} className="text-xs font-semibold text-on-surface">{title}</h4>
      {items.length === 0 ? <p className="mt-1 text-xs text-on-surface-variant">None recorded.</p> : items.map((item) => (
        <p key={item.key} className="mt-1 text-xs leading-relaxed text-on-surface">• {item.text}<SourceLinks label={title} ids={item.sourceSegmentIds} onActivate={jumpToSource} /></p>
      ))}
    </section>
  );

  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
      {summaryStatus && summaryStatus.phase !== 'idle' && (
        <div role="status" className="dialog-card mb-3 p-3 text-xs text-on-surface-variant">
          {summaryBusy
            ? `Generating draft ${summaryStatus.completedChunks} of ${summaryStatus.totalChunks || '…'} · ${formatMeetingTimestamp(summaryStatus.elapsedMs)}`
            : summaryStatus.phase === 'failed'
              ? `Draft generation failed (${summaryStatus.errorCode ?? 'generation_failed'}). Your prior review was kept.`
              : summaryStatus.phase === 'cancelled'
                ? 'Draft generation cancelled. Your prior review was kept.'
                : summaryStatus.phase === 'complete'
                  ? 'Generated draft updated. Any saved review was kept.'
                  : 'Draft generation is stopping…'}
        </div>
      )}

      <MeetingMetadataEditor
        key={detail.session.id}
        session={detail.session}
        disabled={captureBusy}
        onSave={meetings.saveMetadata}
        onApplyCalendar={meetings.applyCalendarEvent}
        onNotice={onNotice}
      />

      {detail.session.retainAudio && meetingAudio ? (
        <MeetingAudioPlayer audio={meetingAudio} captureBusy={captureBusy} />
      ) : !detail.session.retainAudio ? (
        <p className="dialog-card mb-3 p-3 text-xs leading-relaxed text-on-surface-variant">
          Audio was not retained for this meeting. Enable Keep Meeting Audio in Meetings settings to play future sessions.
        </p>
      ) : null}

      <div className="dialog-card mb-3 flex flex-wrap items-end gap-2 p-3">
        <label className="min-w-32 flex-1 text-[11px] font-semibold text-on-surface">Me channel
          <input aria-label="Me speaker label" value={labels.me} maxLength={80} onChange={(event) => setLabelDrafts((current) => ({ ...current, me: event.target.value }))} className="mt-1 w-full rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-low px-2 py-1.5 text-xs" />
        </label>
        <label className="min-w-32 flex-1 text-[11px] font-semibold text-on-surface">Them channel
          <input aria-label="Them speaker label" value={labels.them} maxLength={80} onChange={(event) => setLabelDrafts((current) => ({ ...current, them: event.target.value }))} className="mt-1 w-full rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-low px-2 py-1.5 text-xs" />
        </label>
        {!editing && <button type="button" onClick={() => void saveLabels()} className="dialog-pill-btn px-3 py-2 text-xs text-primary">Save labels</button>}
      </div>

      {remoteSpeakers.length > 0 && (
        <div className="dialog-card mb-3 p-3">
          <h3 className="text-xs font-semibold text-on-surface">Remote speakers</h3>
          <p className="mt-1 text-[11px] text-on-surface-variant">Names apply only to this meeting. Uncertain passages remain {labels.them}.</p>
          <div className="mt-2 grid gap-2 sm:grid-cols-2">
            {remoteSpeakers.map((speaker) => (
              <label key={speaker.speakerId} className="text-[11px] font-semibold text-on-surface">
                Speaker {speaker.speakerId}
                <span className="mt-1 flex gap-2">
                  <input
                    aria-label={`Remote speaker ${speaker.speakerId} label`}
                    value={speaker.label}
                    maxLength={80}
                    onChange={(event) => setRemoteSpeakerDrafts((current) => ({
                      ...current,
                      [speaker.speakerId]: event.target.value,
                    }))}
                    className="min-w-0 flex-1 rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-low px-2 py-1.5 text-xs"
                  />
                  <button type="button" onClick={() => void saveRemoteSpeaker(speaker)} className="dialog-pill-btn px-3 py-1.5 text-xs text-primary">Save</button>
                </span>
              </label>
            ))}
          </div>
        </div>
      )}

      <div className="mb-3 flex flex-wrap items-center gap-2">
        <button type="button" disabled={captureBusy || summaryBusy} onClick={() => void meetings.summarize(detail.session.id)} className="rounded-[var(--ui-radius-pill)] bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))] px-3 py-2 text-xs font-semibold text-on-primary shadow-[var(--ui-shadow-accent)] disabled:opacity-40">
          {detail.generated ? 'Regenerate draft' : 'Generate review draft'}
        </button>
        {summaryBusy && <button type="button" onClick={() => void meetings.cancelSummary()} className="rounded-[var(--ui-radius-control)] px-3 py-2 text-xs font-semibold text-error">{summaryStatus?.phase === 'cancelling' ? 'Cancelling…' : 'Cancel'}</button>}
        {activeDocument && !editing && <button type="button" onClick={beginEdit} className="dialog-pill-btn px-3 py-2 text-xs">Edit review</button>}
        {detail.review?.document && detail.generated && (
          <button type="button" aria-label="Replace review with generated draft" onClick={() => void restore()} className={`rounded-[var(--ui-radius-control)] px-3 py-2 text-xs font-semibold ${restoreConfirm ? 'bg-error-container text-on-error-container' : 'dialog-pill-btn'}`}>
            {restoreConfirm ? 'Confirm replace review' : 'Use generated draft'}
          </button>
        )}
      </div>
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <label className="text-[11px] font-semibold">Format
          <select aria-label="Meeting review export format" value={format} onChange={(event) => {
            const option = MEETING_EXPORT_FORMATS.find((candidate) => candidate.value === event.target.value);
            if (option) setFormat(option.value);
          }} className="ml-1 rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-lowest px-2 py-1.5 text-xs">
            {MEETING_EXPORT_FORMATS.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
          </select>
        </label>
        <button type="button" disabled={captions && detail.session.status === 'active'} onClick={() => void copy()} className="rounded-[var(--ui-radius-control)] px-2 py-1.5 text-xs font-semibold text-primary disabled:opacity-40">{captions ? 'Copy captions' : 'Copy review'}</button>
        <button type="button" disabled={captions && detail.session.status === 'active'} onClick={() => void exportReview()} className="rounded-[var(--ui-radius-control)] px-2 py-1.5 text-xs font-semibold text-primary disabled:opacity-40">Export…</button>
      </div>

      {captions && <p className="mb-3 text-xs text-on-surface-variant" role="status">{detail.session.status === 'active'
        ? 'Stop this meeting before exporting captions.'
        : 'One caption per recorded speech segment, with saved speaker names. Untranscribed sections are marked. Review notes are not included.'}</p>}

      {editing && draft ? (
        <form aria-label="Edit meeting review" aria-busy={saving} onSubmit={(event) => { event.preventDefault(); void saveEdits(); }} className="mb-4 space-y-3 rounded-[var(--ui-radius-card)] border border-primary/25 bg-surface-container-low p-4">
          <fieldset disabled={saving} className="space-y-3">
          <label className="block text-xs font-semibold">Summary<textarea aria-label="Review summary" value={draft.summary.text} onChange={(event) => setDraft({ ...draft, summary: { ...draft.summary, text: event.target.value } })} className="mt-1 min-h-20 w-full rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-lowest p-2 text-xs" /></label>
          {(['decisions', 'openQuestions'] as const).map((section) => <fieldset key={section} className="space-y-2"><legend className="text-xs font-semibold">{section === 'decisions' ? 'Decisions' : 'Open questions'}</legend>{draft[section].length === 0 && <p className="text-xs text-on-surface-variant">None recorded.</p>}{draft[section].map((item, index) => <div key={item.key} className="flex gap-2"><textarea aria-label={`${section} ${index + 1}`} value={item.text} onChange={(event) => setDraft({ ...draft, [section]: draft[section].map((entry) => entry.key === item.key ? { ...entry, text: event.target.value } : entry) })} className="min-h-14 flex-1 rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-lowest p-2 text-xs" /><button type="button" aria-label={`Remove ${section} ${index + 1}`} onClick={() => setDraft({ ...draft, [section]: draft[section].filter((entry) => entry.key !== item.key) })} className="text-xs text-error">Remove</button></div>)}</fieldset>)}
          <fieldset className="space-y-2"><legend className="text-xs font-semibold">Action items</legend>{draft.actionItems.length === 0 && <p className="text-xs text-on-surface-variant">None recorded.</p>}{draft.actionItems.map((item, index) => <div key={item.key} className="grid gap-2 rounded-[var(--ui-radius-control)] bg-surface-container-lowest p-2 sm:grid-cols-[minmax(0,1fr)_8rem_8rem_auto]"><input aria-label={`Action item ${index + 1}`} value={item.text} onChange={(event) => setDraft({ ...draft, actionItems: draft.actionItems.map((entry) => entry.key === item.key ? { ...entry, text: event.target.value } : entry) })} /><input aria-label={`Action owner ${index + 1}`} placeholder="Unknown owner" value={item.owner ?? ''} onChange={(event) => setDraft({ ...draft, actionItems: draft.actionItems.map((entry) => entry.key === item.key ? { ...entry, owner: event.target.value || null } : entry) })} /><input aria-label={`Action due date ${index + 1}`} type="date" value={item.dueDate ?? ''} onChange={(event) => setDraft({ ...draft, actionItems: draft.actionItems.map((entry) => entry.key === item.key ? { ...entry, dueDate: event.target.value || null } : entry) })} /><button type="button" aria-label={`Remove action item ${index + 1}`} onClick={() => setDraft({ ...draft, actionItems: draft.actionItems.filter((entry) => entry.key !== item.key) })} className="text-xs text-error">Remove</button></div>)}</fieldset>
          {newClaims.length > 0 && <fieldset className="space-y-3"><legend className="text-xs font-semibold">New claims · unsaved</legend>{newClaims.map(({ id, claim }) => {
            const label = claim.kind === 'decision' ? 'New decision' : claim.kind === 'action_item' ? 'New action item' : 'New open question';
            const update = (next: NewReviewClaim) => setNewClaims((current) => current.map((entry) => entry.id === id ? { id, claim: next } : entry));
            return <div key={id} className="space-y-2 rounded-[var(--ui-radius-control)] bg-surface-container-lowest p-2">
              <label className="block text-xs font-semibold">{label}<textarea id={`new-meeting-claim-${id}`} aria-label={`${label} text`} value={claim.text} onChange={(event) => update({ ...claim, text: event.target.value })} className="mt-1 min-h-14 w-full rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-lowest p-2 text-xs" /></label>
              {claim.kind === 'action_item' && <div className="flex flex-wrap gap-2"><input aria-label="New action owner" placeholder="Unknown owner" value={claim.owner ?? ''} onChange={(event) => update({ ...claim, owner: event.target.value || null })} className="rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] p-2 text-xs" /><input aria-label="New action due date" type="date" value={claim.dueDate ?? ''} onChange={(event) => update({ ...claim, dueDate: event.target.value || null })} className="rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] p-2 text-xs" /></div>}
              <div className="flex items-center justify-between text-xs"><span>Sources<SourceLinks label={label} ids={claim.sourceSegmentIds} onActivate={jumpToSource} /></span><button type="button" aria-label={`Remove ${label.toLowerCase()}`} onClick={() => setNewClaims((current) => current.filter((entry) => entry.id !== id))} className="text-error">Remove</button></div>
            </div>;
          })}</fieldset>}
          <div className="flex gap-2"><button type="submit" className="rounded-[var(--ui-radius-pill)] bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))] px-3 py-2 text-xs font-semibold text-on-primary shadow-[var(--ui-shadow-accent)]">Save review</button><button type="button" onClick={() => { setEditing(false); setDraft(null); setLabelDrafts({}); setNewClaims([]); setSelectedIds([]); }} className="rounded-[var(--ui-radius-control)] px-3 py-2 text-xs font-semibold">Cancel</button></div>
          </fieldset>
        </form>
      ) : activeDocument ? (
        <article className="mb-4 rounded-[var(--ui-radius-card)] border border-primary/25 bg-[var(--ui-tint-accent-subtle)] p-4">
          <div className="flex items-center justify-between gap-2"><h3 className="text-sm font-semibold">Meeting review</h3><span className="text-[10px] font-semibold uppercase tracking-wide text-on-surface-variant">{detail.activeOrigin === 'reviewed' ? 'Reviewed' : 'Generated draft'}</span></div>
          <p className="mt-2 text-xs leading-relaxed">{activeDocument.summary.text}<SourceLinks label="Summary" ids={activeDocument.summary.sourceSegmentIds} onActivate={jumpToSource} /></p>
          {renderTextItems('Decisions', activeDocument.decisions)}
          <section className="mt-3"><h4 className="text-xs font-semibold">Action items</h4>{activeDocument.actionItems.length === 0 ? <p className="mt-1 text-xs text-on-surface-variant">None recorded.</p> : activeDocument.actionItems.map((item) => <p key={item.key} className="mt-1 text-xs">• {item.text} — {item.owner ?? 'Unknown'} · {item.dueDate ?? 'Unknown'}<SourceLinks label="Action item" ids={item.sourceSegmentIds} onActivate={jumpToSource} /></p>)}</section>
          {renderTextItems('Open questions', activeDocument.openQuestions)}
        </article>
      ) : <div className="mb-4 rounded-[var(--ui-radius-card)] border border-dashed border-[var(--ui-hairline-strong)] p-5 text-center"><p className="text-sm font-semibold">No review draft yet</p><p className="mt-1 text-xs text-on-surface-variant">Generate one locally from the completed transcript. Nothing is sent to the cloud.</p></div>}

      <section aria-labelledby="meeting-transcript-title">
        {activeDocument && <div className="dialog-card mb-3 flex flex-wrap items-center gap-2 p-3">
          <span role="status" className="text-xs text-on-surface-variant">{selectedIds.length} transcript sources selected</span>
          {(['decision', 'action_item', 'open_question'] as const).map((kind) => <button key={kind} type="button" disabled={captureBusy || saving} onClick={() => createClaim(kind)} className="dialog-pill-btn px-3 py-2 text-xs disabled:opacity-40">{kind === 'decision' ? 'New decision' : kind === 'action_item' ? 'New action item' : 'New open question'}</button>)}
          {selectedIds.length > 0 && <button type="button" disabled={saving} onClick={() => setSelectedIds([])} className="dialog-pill-btn px-3 py-2 text-xs">Clear selection</button>}
        </div>}
        <h3 id="meeting-transcript-title" className="mb-1 text-sm font-semibold">Transcript evidence</h3><p className="mb-2 text-[11px] text-on-surface-variant">Select transcript sources to add a claim to your review. Raw text and canonical Me/Them channels are never changed by review edits.</p>{segments.length === 0 ? <p className="py-8 text-center text-xs text-on-surface-variant">No speech segments were saved.</p> : segments.map((segment) => <TranscriptRow key={segment.id} segment={segment} selected={selectedIds.includes(segment.id)} onSelect={() => setSelectedIds((current) => current.includes(segment.id) ? current.filter((id) => id !== segment.id) : [...current, segment.id])} selectionDisabled={!activeDocument || captureBusy || saving || segment.status !== 'final' || !segment.text.trim()} labels={labels} remoteSpeakers={remoteSpeakers} onPlay={detail.session.retainAudio && meetingAudio && segment.audioAvailable ? () => meetingAudio.playSegment({ speaker: segment.speaker, startMs: segment.startMs }) : undefined} playbackDisabled={captureBusy || meetingAudio?.status === 'loading' || meetingAudio?.status === 'buffering' || meetingAudio?.status === 'unavailable' || meetingAudio?.status === 'error'} />)}</section>
    </div>
  );
}
