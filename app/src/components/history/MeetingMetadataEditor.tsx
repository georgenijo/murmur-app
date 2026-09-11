import { useEffect, useRef, useState } from 'react';
import type { MeetingSession, SaveMeetingMetadataRequest } from '../../lib/meetings';
import { MeetingCalendarNaming } from './MeetingCalendarNaming';

interface MeetingMetadataEditorProps {
  session: MeetingSession;
  disabled: boolean;
  onSave: (request: SaveMeetingMetadataRequest) => Promise<boolean>;
  onApplyCalendar: (sessionId: string, selectionToken: string) => Promise<boolean>;
  onNotice: (message: string) => void;
}

export function MeetingMetadataEditor({ session, disabled, onSave, onApplyCalendar, onNotice }: MeetingMetadataEditorProps) {
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

  return (
    <section className="dialog-card mb-3 p-3" aria-label="Meeting details">
      {draft ? (
        <form aria-label="Name meeting" aria-busy={saving} onSubmit={(event) => { event.preventDefault(); void save(); }}>
          <fieldset disabled={disabled || saving || applyingCalendar} className="space-y-2">
            <label className="block text-xs font-semibold text-on-surface">Meeting title
              <input autoFocus value={title} maxLength={200} onChange={(event) => setDraft({ ...draft, title: event.target.value })} placeholder="Untitled meeting" className="mt-1 w-full rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-low px-2 py-1.5 text-xs focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary" />
            </label>
            <label className="block text-xs font-semibold text-on-surface">Attendees, one per line
              <textarea value={attendees} maxLength={20100} rows={2} onChange={(event) => setDraft({ ...draft, attendees: event.target.value })} className="mt-1 w-full resize-y rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-low px-2 py-1.5 text-xs focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary" />
            </label>
            <div className="flex gap-2">
              <button type="submit" className="dialog-pill-btn px-3 py-1.5 text-xs text-primary">{saving ? 'Saving…' : 'Save details'}</button>
              <button type="button" onClick={() => setDraft(null)} className="dialog-pill-btn px-3 py-1.5 text-xs">Cancel naming</button>
            </div>
          </fieldset>
        </form>
      ) : (
        <>
          <div className="flex items-center gap-2">
            <h3 className="min-w-0 flex-1 break-words text-sm font-semibold text-on-surface">{session.title || 'Untitled meeting'}</h3>
            <button type="button" disabled={disabled || applyingCalendar} onClick={beginEdit} className="dialog-pill-btn shrink-0 px-3 py-1.5 text-xs text-primary disabled:opacity-40">{session.title ? 'Rename' : 'Name meeting'}</button>
          </div>
          {session.attendees.length > 0 && <p className="mt-1 break-words text-xs text-on-surface-variant">Attendees: {session.attendees.join(', ')}</p>}
          {session.titleSource && <p className="mt-1 text-xs text-on-surface-variant">{session.titleSource === 'calendar' ? 'Named from calendar' : session.titleSource === 'generated' ? 'Generated title' : 'Manually named'}</p>}
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
    </section>
  );
}
