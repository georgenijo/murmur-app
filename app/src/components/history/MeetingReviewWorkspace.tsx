import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import { ChevronDown } from 'lucide-react';
import { MeetingHeaderActions } from './MeetingHeaderActions';
import { meetingCapturePresentation } from './meetingCapturePresentation';
import { MeetingMetadataEditor } from './MeetingMetadataEditor';
import {
  formatBundleSize,
  formatMeetingTimestamp,
  MEETING_EXPORT_FORMATS,
  meetingSegmentDisplayLabel,
  type MeetingBundlePreview,
  type MeetingReviewExportFormat,
  type MeetingSegment,
  type MeetingSpeakerLabels,
  type RemoteSpeakerLabel,
} from '../../lib/meetings';
import type { MeetingAudioController } from '../../lib/hooks/useMeetingAudio';
import { MeetingAudioPlayer } from './MeetingAudioPlayer';

interface MeetingReviewWorkspaceProps {
  meetings: ReturnType<typeof useMeetings>;
  segments: MeetingSegment[];
  captureBusy: boolean;
  meetingAudio?: MeetingAudioController;
  onNotice: (message: string) => void;
  /** Session facts shown under the meeting title. */
  headerMeta?: string;
  onDelete?: () => void;
  onDisarmDelete?: () => void;
  confirmingDelete?: boolean;
}

interface WorkspaceActivation {
  readonly sessionId: string;
}

function TranscriptRow({ segment, labels, remoteSpeakers, onPlay, playbackDisabled }: {
  segment: MeetingSegment;
  labels: { me: string; them: string };
  remoteSpeakers: RemoteSpeakerLabel[];
  onPlay?: () => void;
  playbackDisabled?: boolean;
}) {
  const canonical = segment.speaker === 'me' ? 'Me' : 'Them';
  const display = meetingSegmentDisplayLabel(segment, labels, remoteSpeakers);
  return (
    <article
      id={`meeting-segment-${segment.id}`}
      tabIndex={-1}
      aria-label={`${canonical} channel, ${display}, at ${formatMeetingTimestamp(segment.startMs)}`}
      className="meeting-transcript-row"
    >
      {onPlay ? (
        <button
          type="button"
          disabled={playbackDisabled}
          aria-label={`Play segment at ${formatMeetingTimestamp(segment.startMs)}, ${canonical} channel`}
          onClick={onPlay}
          className="meeting-transcript-time"
          data-playable="true"
        >
          <span className="meeting-segment-play" aria-hidden="true">▶</span>{formatMeetingTimestamp(segment.startMs)}
        </button>
      ) : (
        <span className="meeting-transcript-time">{formatMeetingTimestamp(segment.startMs)}</span>
      )}
      <span className="meeting-transcript-speaker" data-speaker={segment.speaker} title={`${display} (${canonical})`}>
        {display}{display !== canonical && <span className="font-normal text-on-surface-variant"> ({canonical})</span>}
      </span>
      <div className="flex min-w-0 items-start gap-2">
        <p className="min-w-0 flex-1 whitespace-pre-wrap break-words text-on-surface">
          {segment.status === 'final' ? segment.text : segment.status === 'pending' ? 'Transcript pending…' : 'Transcription failed.'}
        </p>
      </div>
    </article>
  );
}

export function MeetingReviewWorkspace({ meetings, segments, captureBusy, meetingAudio, onNotice, headerMeta, onDelete, onDisarmDelete, confirmingDelete }: MeetingReviewWorkspaceProps) {
  const detail = meetings.detail!;
  const capture = meetingCapturePresentation(detail.session.id, meetings.status);
  const activeWorkspace = useRef<WorkspaceActivation | null>(null);
  const [labelDrafts, setLabelDrafts] = useState<Partial<MeetingSpeakerLabels>>({});
  const [remoteSpeakerDrafts, setRemoteSpeakerDrafts] = useState<Partial<Record<number, string>>>({});
  const speakersRef = useRef<HTMLDetailsElement>(null);
  const [format, setFormat] = useState<MeetingReviewExportFormat>('markdown');
  const captions = format === 'srt' || format === 'vtt';
  const [bundlePlan, setBundlePlan] = useState<{ sessionId: string; preview: MeetingBundlePreview } | null>(null);
  const [bundleBusy, setBundleBusy] = useState(false);
  const labels = {
    me: labelDrafts.me ?? detail.labels.me,
    them: labelDrafts.them ?? detail.labels.them,
  };
  const remoteSpeakers = detail.remoteSpeakers.map((speaker) => ({
    ...speaker,
    label: remoteSpeakerDrafts[speaker.speakerId] ?? speaker.label,
  }));

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

  const copy = async (selectedFormat: MeetingReviewExportFormat = 'markdown') => {
    const activation = activeWorkspace.current;
    const sessionId = detail.session.id;
    if (await meetings.copy(sessionId, selectedFormat) && isCurrentWorkspace(activation)) {
      onNotice(`Meeting ${selectedFormat === 'srt' || selectedFormat === 'vtt' ? 'captions' : 'transcript'} copied as ${selectedFormat}.`);
    }
  };

  const exportReview = async (selectedFormat: MeetingReviewExportFormat) => {
    setFormat(selectedFormat);
    const activation = activeWorkspace.current;
    const sessionId = detail.session.id;
    const path = await meetings.exportReview(sessionId, detail.session.startedAtMs, selectedFormat);
    if (path && isCurrentWorkspace(activation)) onNotice(`Meeting ${selectedFormat === 'srt' || selectedFormat === 'vtt' ? 'captions' : 'transcript'} exported as ${selectedFormat}.`);
  };

  const planBundle = async () => {
    const activation = activeWorkspace.current;
    const sessionId = detail.session.id;
    setBundleBusy(true);
    const preview = await meetings.previewBundle(sessionId);
    setBundleBusy(false);
    if (preview && isCurrentWorkspace(activation)) setBundlePlan({ sessionId, preview });
  };

  const exportBundle = async () => {
    const activation = activeWorkspace.current;
    const sessionId = detail.session.id;
    setBundleBusy(true);
    const saved = await meetings.exportBundle(sessionId, detail.session.startedAtMs, format);
    setBundleBusy(false);
    if (!saved || !isCurrentWorkspace(activation)) return;
    setBundlePlan(null);
    const { includedSegments, omittedSegments } = saved.receipt;
    onNotice(omittedSegments > 0
      ? `Meeting exported with ${includedSegments} audio segments; ${omittedSegments} unavailable segments are listed in manifest.json.`
      : `Meeting exported with ${includedSegments} audio segments.`);
  };

  const speakerSummary = [labels.me, labels.them, ...remoteSpeakers.map((speaker) => speaker.label)].filter(Boolean).join(', ');

  return (
    <div className="meeting-workspace">
      <MeetingMetadataEditor
        key={detail.session.id}
        session={detail.session}
        disabled={captureBusy}
        onSave={meetings.saveMetadata}
        onApplyCalendar={meetings.applyCalendarEvent}
        onNotice={onNotice}
        meta={headerMeta}
        renderActions={(rename, calendar, calendarDisabled) => <MeetingHeaderActions
          active={detail.session.status === 'active'} busy={captureBusy} calendarDisabled={calendarDisabled}
          retainedAudio={detail.session.retainAudio} bundleBusy={bundleBusy} confirmingDelete={confirmingDelete}
          onRename={rename} onCalendar={calendar} onDelete={onDelete} onDisarmDelete={onDisarmDelete}
          onSpeakers={() => { if (speakersRef.current) { speakersRef.current.open = true; speakersRef.current.querySelector('input')?.focus(); } }}
          onCopy={(value) => void copy(value)} onExport={(value) => void exportReview(value)} onBundle={() => void planBundle()}
        />}
      />

      {detail.session.retainAudio && meetingAudio ? (
        <MeetingAudioPlayer audio={meetingAudio} captureBusy={captureBusy} />
      ) : !detail.session.retainAudio ? (
        <p className="meeting-quiet-note">
          Audio was not retained for this meeting. Enable Keep Meeting Audio in Meetings settings to play future sessions.
        </p>
      ) : null}

      {bundlePlan && bundlePlan.sessionId === detail.session.id && (
        <section aria-label="Export with audio" className="meeting-quiet-note leading-relaxed">
          <label className="meeting-export-format">Format
            <select aria-label="Meeting audio bundle format" value={format} disabled={bundleBusy} onChange={(event) => {
              const option = MEETING_EXPORT_FORMATS.find((candidate) => candidate.value === event.target.value);
              if (option) setFormat(option.value);
            }}>
              {MEETING_EXPORT_FORMATS.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
            </select>
          </label>
          <p role="status">
            The .zip holds the {captions ? 'captions' : 'transcript'} as {format}, {bundlePlan.preview.audioSegments} audio segments ({formatBundleSize(bundlePlan.preview.audioBytes)}), and a manifest. Audio is saved exactly as recorded.
          </p>
          {bundlePlan.preview.unavailableSegments > 0 && (
            <p role="alert" className="mt-1 text-error">
              {bundlePlan.preview.unavailableSegments} retained audio segments are missing and will be left out. The manifest lists them, so the bundle is incomplete.
            </p>
          )}
          {bundlePlan.preview.audioBytes > bundlePlan.preview.maxAudioBytes && (
            <p role="alert" className="mt-1 text-error">
              This is over the {formatBundleSize(bundlePlan.preview.maxAudioBytes)} bundle limit. Use Export… for the {captions ? 'captions' : 'transcript'} on its own.
            </p>
          )}
          <div className="mt-2 flex gap-2">
            <button type="button" disabled={bundleBusy || captureBusy || bundlePlan.preview.audioBytes > bundlePlan.preview.maxAudioBytes} onClick={() => void exportBundle()} className="ui-form-button meeting-header-action" data-tone="accent">Choose location…</button>
            <button type="button" disabled={bundleBusy} onClick={() => setBundlePlan(null)} className="ui-form-button meeting-header-action">Cancel</button>
          </div>
        </section>
      )}

      {captions && <p className="meeting-quiet-note" role="status">{detail.session.status === 'active'
        ? capture?.phase === 'recording' ? 'Stop this meeting before exporting captions.' : capture?.footer ?? 'Captions will be available when this meeting finishes.'
        : 'One caption per recorded speech segment, with saved speaker names. Untranscribed sections are marked. Review notes are not included.'}</p>}

      <section className="meeting-transcript" aria-labelledby="meeting-transcript-title">
        <div className="meeting-transcript-header">
          <h3 id="meeting-transcript-title">Transcript</h3>
          <span className="meeting-transcript-count">{segments.length} {segments.length === 1 ? 'segment' : 'segments'}</span>
          <span className="flex-1" />
          <details ref={speakersRef} className="meeting-speakers">
            <summary className="ui-form-button meeting-header-action">Speakers: {speakerSummary}<ChevronDown aria-hidden="true" className="h-3.5 w-3.5" /></summary>
            <div className="meeting-speakers-panel">
              <div className="flex flex-wrap items-end gap-2">
                <label className="min-w-32 flex-1 text-(length:--ui-font-caption) font-semibold text-on-surface">Me channel
                  <input aria-label="Me speaker label" value={labels.me} maxLength={80} onChange={(event) => setLabelDrafts((current) => ({ ...current, me: event.target.value }))} className="meeting-field mt-1" />
                </label>
                <label className="min-w-32 flex-1 text-(length:--ui-font-caption) font-semibold text-on-surface">Them channel
                  <input aria-label="Them speaker label" value={labels.them} maxLength={80} onChange={(event) => setLabelDrafts((current) => ({ ...current, them: event.target.value }))} className="meeting-field mt-1" />
                </label>
                {<button type="button" onClick={() => void saveLabels()} className="ui-form-button meeting-header-action" data-tone="accent">Save labels</button>}
              </div>
              {remoteSpeakers.length > 0 && (
                <div className="mt-3 border-t border-[var(--ui-hairline)] pt-3">
                  <h4 className="text-xs font-semibold text-on-surface">Remote speakers</h4>
                  <p className="mt-1 text-(length:--ui-font-caption) text-on-surface-variant">Names apply only to this meeting. Uncertain passages remain {labels.them}.</p>
                  <div className="mt-2 grid gap-2 sm:grid-cols-2">
                    {remoteSpeakers.map((speaker) => (
                      <label key={speaker.speakerId} className="text-(length:--ui-font-caption) font-semibold text-on-surface">
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
                            className="meeting-field min-w-0 flex-1"
                          />
                          <button type="button" onClick={() => void saveRemoteSpeaker(speaker)} className="ui-form-button meeting-header-action" data-tone="accent">Save</button>
                        </span>
                      </label>
                    ))}
                  </div>
                </div>
              )}
            </div>
          </details>
        </div>
        {segments.length === 0 ? <p className="py-8 text-center text-xs text-on-surface-variant">{capture?.footer ?? 'No speech segments were saved.'}</p> : segments.map((segment) => <TranscriptRow key={segment.id} segment={segment} labels={labels} remoteSpeakers={remoteSpeakers} onPlay={detail.session.retainAudio && meetingAudio && segment.audioAvailable ? () => meetingAudio.playSegment({ speaker: segment.speaker, startMs: segment.startMs }) : undefined} playbackDisabled={captureBusy || meetingAudio?.status === 'loading' || meetingAudio?.status === 'buffering' || meetingAudio?.status === 'unavailable' || meetingAudio?.status === 'error'} />)}
        {segments.length > 0 && capture && <p className="meeting-listening" role="status">{capture.footer}</p>}
      </section>
    </div>
  );
}
