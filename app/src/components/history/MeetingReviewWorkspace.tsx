import { useEffect, useMemo, useState } from 'react';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import {
  formatMeetingTimestamp,
  type EditableReviewDocument,
  type MeetingReviewDocumentV1,
  type MeetingReviewExportFormat,
  type MeetingSegment,
  type ReviewEditBase,
} from '../../lib/meetings';

interface MeetingReviewWorkspaceProps {
  meetings: ReturnType<typeof useMeetings>;
  segments: MeetingSegment[];
  captureBusy: boolean;
  onNotice: (message: string) => void;
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
          className="rounded bg-surface-container-high px-1.5 py-0.5 text-[10px] font-semibold text-primary hover:brightness-95 focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary"
        >
          #{id}
        </button>
      ))}
    </span>
  );
}

function TranscriptRow({ segment, labels }: {
  segment: MeetingSegment;
  labels: { me: string; them: string };
}) {
  const canonical = segment.speaker === 'me' ? 'Me' : 'Them';
  const display = segment.speaker === 'me' ? labels.me : labels.them;
  return (
    <article
      id={`meeting-segment-${segment.id}`}
      tabIndex={-1}
      aria-label={`${canonical} channel, ${display}, at ${formatMeetingTimestamp(segment.startMs)}`}
      className="grid scroll-m-20 grid-cols-[3.25rem_7rem_minmax(0,1fr)] gap-2 border-b border-outline-variant/10 py-2 text-sm last:border-0 focus-visible:rounded-lg focus-visible:bg-primary-container/30 focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary"
    >
      <span className="font-mono text-[11px] tabular-nums text-on-surface-variant">{formatMeetingTimestamp(segment.startMs)}</span>
      <span className={`truncate text-xs font-bold ${segment.speaker === 'me' ? 'text-primary' : 'text-success'}`} title={`${display} (${canonical})`}>
        {display} <span className="font-normal text-on-surface-variant">({canonical})</span>
      </span>
      <p className="min-w-0 whitespace-pre-wrap break-words text-on-surface">
        {segment.status === 'final' ? segment.text : segment.status === 'pending' ? 'Transcript pending…' : 'Transcription failed.'}
      </p>
    </article>
  );
}

export function MeetingReviewWorkspace({ meetings, segments, captureBusy, onNotice }: MeetingReviewWorkspaceProps) {
  const detail = meetings.detail!;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<EditableReviewDocument | null>(null);
  const [labels, setLabels] = useState(detail.labels);
  const [format, setFormat] = useState<MeetingReviewExportFormat>('markdown');
  const [restoreConfirm, setRestoreConfirm] = useState(false);
  const summaryStatus = meetings.summaryStatus.sessionId === detail.session.id ? meetings.summaryStatus : null;
  const summaryBusy = summaryStatus?.phase === 'running' || summaryStatus?.phase === 'cancelling';
  const activeDocument = detail.activeDocument;
  const sourceById = useMemo(() => new Map(segments.map((segment) => [segment.id, segment])), [segments]);

  useEffect(() => {
    setLabels(detail.labels);
    setEditing(false);
    setDraft(null);
    setRestoreConfirm(false);
  }, [detail.session.id, detail.review?.revision, detail.generated?.revision]);

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
    const saved = await meetings.saveReview({
      sessionId: detail.session.id,
      expectedReviewRevision: detail.review?.revision ?? null,
      base: { kind: 'labels_only' },
      labels,
      document: null,
    });
    if (saved) onNotice('Speaker labels saved on this Mac.');
  };

  const saveEdits = async () => {
    const base: ReviewEditBase = detail.activeOrigin === 'reviewed' && detail.review
      ? { kind: 'review', reviewRevision: detail.review.revision }
      : detail.generated
        ? { kind: 'generated', generatedRevision: detail.generated.revision }
        : { kind: 'labels_only' };
    const saved = await meetings.saveReview({
      sessionId: detail.session.id,
      expectedReviewRevision: detail.review?.revision ?? null,
      base,
      labels,
      document: draft ?? (activeDocument ? editable(activeDocument) : null),
    });
    if (saved) {
      setEditing(false);
      onNotice('Meeting review saved on this Mac.');
    }
  };

  const restore = async () => {
    if (!detail.generated) return;
    if (!restoreConfirm) {
      setRestoreConfirm(true);
      return;
    }
    setRestoreConfirm(false);
    if (await meetings.restoreReview(detail.session.id, detail.generated.revision, detail.review?.revision ?? null)) {
      onNotice('Review replaced with the generated draft. Raw transcript evidence was unchanged.');
    }
  };

  const copy = async () => {
    if (await meetings.copy(detail.session.id, format)) onNotice(`Meeting review copied as ${format}.`);
  };

  const exportReview = async () => {
    const path = await meetings.exportReview(detail.session.id, detail.session.startedAtMs, format);
    if (path) onNotice(`Meeting review exported as ${format}.`);
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
        <div role="status" className="mb-3 rounded-xl border border-outline-variant/20 bg-surface-container-low p-3 text-xs text-on-surface-variant">
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

      <div className="mb-3 flex flex-wrap items-end gap-2 rounded-xl border border-outline-variant/20 bg-surface-container-lowest p-3">
        <label className="min-w-32 flex-1 text-[11px] font-semibold text-on-surface">Me channel
          <input aria-label="Me speaker label" value={labels.me} maxLength={80} onChange={(event) => setLabels({ ...labels, me: event.target.value })} className="mt-1 w-full rounded-lg border border-outline-variant bg-surface-container-low px-2 py-1.5 text-xs" />
        </label>
        <label className="min-w-32 flex-1 text-[11px] font-semibold text-on-surface">Them channel
          <input aria-label="Them speaker label" value={labels.them} maxLength={80} onChange={(event) => setLabels({ ...labels, them: event.target.value })} className="mt-1 w-full rounded-lg border border-outline-variant bg-surface-container-low px-2 py-1.5 text-xs" />
        </label>
        {!editing && <button type="button" onClick={() => void saveLabels()} className="rounded-lg bg-surface-container-high px-3 py-2 text-xs font-semibold text-primary">Save labels</button>}
      </div>

      <div className="mb-3 flex flex-wrap items-center gap-2">
        <button type="button" disabled={captureBusy || summaryBusy} onClick={() => void meetings.summarize(detail.session.id)} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary disabled:opacity-40">
          {detail.generated ? 'Regenerate draft' : 'Generate review draft'}
        </button>
        {summaryBusy && <button type="button" onClick={() => void meetings.cancelSummary()} className="rounded-lg px-3 py-2 text-xs font-semibold text-error">{summaryStatus?.phase === 'cancelling' ? 'Cancelling…' : 'Cancel'}</button>}
        {activeDocument && !editing && <button type="button" onClick={beginEdit} className="rounded-lg border border-outline-variant px-3 py-2 text-xs font-semibold">Edit review</button>}
        {detail.review?.document && detail.generated && (
          <button type="button" aria-label="Replace review with generated draft" onClick={() => void restore()} className={`rounded-lg px-3 py-2 text-xs font-semibold ${restoreConfirm ? 'bg-error-container text-on-error-container' : 'border border-outline-variant'}`}>
            {restoreConfirm ? 'Confirm replace review' : 'Use generated draft'}
          </button>
        )}
        <label className="ml-auto text-[11px] font-semibold">Format
          <select aria-label="Meeting review export format" value={format} onChange={(event) => setFormat(event.target.value as MeetingReviewExportFormat)} className="ml-1 rounded-lg border border-outline-variant bg-surface-container-lowest px-2 py-1.5 text-xs">
            <option value="markdown">Markdown</option><option value="text">Plain text</option><option value="json">JSON</option>
          </select>
        </label>
        <button type="button" onClick={() => void copy()} className="rounded-lg px-2 py-1.5 text-xs font-semibold text-primary">Copy review</button>
        <button type="button" onClick={() => void exportReview()} className="rounded-lg px-2 py-1.5 text-xs font-semibold text-primary">Export…</button>
      </div>

      {editing && draft ? (
        <form aria-label="Edit meeting review" aria-busy={false} onSubmit={(event) => { event.preventDefault(); void saveEdits(); }} className="mb-4 space-y-3 rounded-xl border border-primary/25 bg-surface-container-low p-4">
          <label className="block text-xs font-semibold">Summary<textarea aria-label="Review summary" value={draft.summary.text} onChange={(event) => setDraft({ ...draft, summary: { ...draft.summary, text: event.target.value } })} className="mt-1 min-h-20 w-full rounded-lg border border-outline-variant bg-surface-container-lowest p-2 text-xs" /></label>
          {(['decisions', 'openQuestions'] as const).map((section) => <fieldset key={section} className="space-y-2"><legend className="text-xs font-semibold">{section === 'decisions' ? 'Decisions' : 'Open questions'}</legend>{draft[section].length === 0 && <p className="text-xs text-on-surface-variant">None recorded.</p>}{draft[section].map((item, index) => <div key={item.key} className="flex gap-2"><textarea aria-label={`${section} ${index + 1}`} value={item.text} onChange={(event) => setDraft({ ...draft, [section]: draft[section].map((entry) => entry.key === item.key ? { ...entry, text: event.target.value } : entry) })} className="min-h-14 flex-1 rounded-lg border border-outline-variant bg-surface-container-lowest p-2 text-xs" /><button type="button" aria-label={`Remove ${section} ${index + 1}`} onClick={() => setDraft({ ...draft, [section]: draft[section].filter((entry) => entry.key !== item.key) })} className="text-xs text-error">Remove</button></div>)}</fieldset>)}
          <fieldset className="space-y-2"><legend className="text-xs font-semibold">Action items</legend>{draft.actionItems.length === 0 && <p className="text-xs text-on-surface-variant">None recorded.</p>}{draft.actionItems.map((item, index) => <div key={item.key} className="grid gap-2 rounded-lg bg-surface-container-lowest p-2 sm:grid-cols-[minmax(0,1fr)_8rem_8rem_auto]"><input aria-label={`Action item ${index + 1}`} value={item.text} onChange={(event) => setDraft({ ...draft, actionItems: draft.actionItems.map((entry) => entry.key === item.key ? { ...entry, text: event.target.value } : entry) })} /><input aria-label={`Action owner ${index + 1}`} placeholder="Unknown owner" value={item.owner ?? ''} onChange={(event) => setDraft({ ...draft, actionItems: draft.actionItems.map((entry) => entry.key === item.key ? { ...entry, owner: event.target.value || null } : entry) })} /><input aria-label={`Action due date ${index + 1}`} type="date" value={item.dueDate ?? ''} onChange={(event) => setDraft({ ...draft, actionItems: draft.actionItems.map((entry) => entry.key === item.key ? { ...entry, dueDate: event.target.value || null } : entry) })} /><button type="button" aria-label={`Remove action item ${index + 1}`} onClick={() => setDraft({ ...draft, actionItems: draft.actionItems.filter((entry) => entry.key !== item.key) })} className="text-xs text-error">Remove</button></div>)}</fieldset>
          <div className="flex gap-2"><button type="submit" className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary">Save review</button><button type="button" onClick={() => { setEditing(false); setDraft(null); setLabels(detail.labels); }} className="rounded-lg px-3 py-2 text-xs font-semibold">Cancel</button></div>
        </form>
      ) : activeDocument ? (
        <article className="mb-4 rounded-xl border border-primary/20 bg-surface-container-low p-4">
          <div className="flex items-center justify-between gap-2"><h3 className="text-sm font-semibold">Meeting review</h3><span className="text-[10px] font-semibold uppercase tracking-wide text-on-surface-variant">{detail.activeOrigin === 'reviewed' ? 'Reviewed' : 'Generated draft'}</span></div>
          <p className="mt-2 text-xs leading-relaxed">{activeDocument.summary.text}<SourceLinks label="Summary" ids={activeDocument.summary.sourceSegmentIds} onActivate={jumpToSource} /></p>
          {renderTextItems('Decisions', activeDocument.decisions)}
          <section className="mt-3"><h4 className="text-xs font-semibold">Action items</h4>{activeDocument.actionItems.length === 0 ? <p className="mt-1 text-xs text-on-surface-variant">None recorded.</p> : activeDocument.actionItems.map((item) => <p key={item.key} className="mt-1 text-xs">• {item.text} — {item.owner ?? 'Unknown'} · {item.dueDate ?? 'Unknown'}<SourceLinks label="Action item" ids={item.sourceSegmentIds} onActivate={jumpToSource} /></p>)}</section>
          {renderTextItems('Open questions', activeDocument.openQuestions)}
        </article>
      ) : <div className="mb-4 rounded-xl border border-dashed border-outline-variant p-5 text-center"><p className="text-sm font-semibold">No review draft yet</p><p className="mt-1 text-xs text-on-surface-variant">Generate one locally from the completed transcript. Nothing is sent to the cloud.</p></div>}

      <section aria-labelledby="meeting-transcript-title"><h3 id="meeting-transcript-title" className="mb-1 text-sm font-semibold">Transcript evidence</h3><p className="mb-2 text-[11px] text-on-surface-variant">Raw segment text and canonical Me/Them channels are never changed by review edits.</p>{segments.length === 0 ? <p className="py-8 text-center text-xs text-on-surface-variant">No speech segments were saved.</p> : segments.map((segment) => <TranscriptRow key={segment.id} segment={segment} labels={labels} />)}</section>
    </div>
  );
}
