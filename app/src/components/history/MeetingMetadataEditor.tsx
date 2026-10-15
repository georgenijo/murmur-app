import { useEffect, useRef, useState, type ReactNode } from 'react';
import type { MeetingSession, SaveMeetingMetadataRequest } from '../../lib/meetings';
import { MeetingCalendarNaming } from './MeetingCalendarNaming';

interface MeetingMetadataEditorProps {
  session: MeetingSession;
  disabled: boolean;
  onSave: (request: SaveMeetingMetadataRequest) => Promise<boolean>;
  onApplyCalendar: (sessionId: string, selectionToken: string) => Promise<boolean>;
  onNotice: (message: string) => void;
  /** One-line session facts shown under the title (date, duration, audio). */
  meta?: string;
  /** Extra header actions, such as Delete, placed after the naming actions. */
  actions?: ReactNode;
}

/** Meeting detail header: title, facts, and naming actions in one block. */
export function MeetingMetadataEditor({ session, disabled, onSave, onApplyCalendar, onNotice, meta, actions }: MeetingMetadataEditorProps) {
  const [draft, setDraft] = useState<{ title: string; attendees: string } | null>(null);
  const title = draft?.title ?? session.title ?? '';
  const attendees = draft?.attendees ?? session.attendees.join('\n');
  const [saving, setSaving] = useState(false);
  const [applyingCalendar, setApplyingCalendar] = useState(false);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const beginEdit = () => {
    setDraft((current) => current ?? { title: session.title ?? '', attendees: session.attendees.join('\n') });
  };

  const save = async () => {
    setSaving(true);
    const saved = await onSave({
      sessionId: session.id,
      title: title.trim() || null,
      attendees: attendees.split('\n').map((name) => name.trim()).filter(Boolean),
    });
    if (!mounted.current) return;
    setSaving(false);
    if (saved) {
      setDraft(null);
      onNotice('Meeting details saved on this Mac.');
    }
  };

  const titleSource = session.titleSource === 'calendar'
    ? 'Named from calendar'
    : session.titleSource === 'generated'
      ? 'Generated title'
      : session.titleSource === 'manual'
        ? 'Manually named'
        : null;

  return (
    <section className="meeting-detail-header" aria-label="Meeting details">
      {draft ? (
        <form className="meeting-detail-header-form" aria-label="Name meeting" aria-busy={saving} onSubmit={(event) => { event.preventDefault(); void save(); }}>
          <fieldset disabled={disabled || saving || applyingCalendar} className="space-y-2">
            <label className="block text-xs font-semibold text-on-surface">Meeting title
              <input autoFocus value={title} maxLength={200} onChange={(event) => setDraft({ ...draft, title: event.target.value })} placeholder="Untitled meeting" className="meeting-field mt-1" />
            </label>
            <label className="block text-xs font-semibold text-on-surface">Attendees, one per line
              <textarea value={attendees} maxLength={20100} rows={2} onChange={(event) => setDraft({ ...draft, attendees: event.target.value })} className="meeting-field mt-1 resize-y" />
            </label>
            <div className="flex gap-2">
              <button type="submit" className="meeting-header-action" data-tone="accent">{saving ? 'Saving…' : 'Save details'}</button>
              <button type="button" onClick={() => setDraft(null)} className="meeting-header-action">Cancel naming</button>
            </div>
          </fieldset>
        </form>
      ) : (
        <>
          <h3 className="meeting-detail-title" data-untitled={!session.title}>{session.title || 'Untitled meeting'}</h3>
          {(meta || session.attendees.length > 0 || titleSource) && (
            <div className="meeting-detail-meta">
              {meta && <p>{meta}</p>}
              {(session.attendees.length > 0 || titleSource) && (
                <p>
                  {session.attendees.length > 0 && <>Attendees: {session.attendees.join(', ')}</>}
                  {session.attendees.length > 0 && titleSource && ' · '}
                  {titleSource}
                </p>
              )}
            </div>
          )}
          <button type="button" disabled={disabled || applyingCalendar} onClick={beginEdit} className="meeting-header-action">{session.title ? 'Rename' : 'Name meeting'}</button>
        </>
      )}
      <MeetingCalendarNaming
        sessionId={session.id}
        disabled={disabled || saving}
        onApply={async (sessionId, selectionToken) => {
          setApplyingCalendar(true);
          const applied = await onApplyCalendar(sessionId, selectionToken);
          if (mounted.current) {
            setApplyingCalendar(false);
            if (applied) setDraft(null);
          }
          return applied;
        }}
        onManualNaming={beginEdit}
        onNotice={onNotice}
      />
      {actions}
    </section>
  );
}
