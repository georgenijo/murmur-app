import { useEffect, useRef, useState } from 'react';
import type { MeetingSession, SaveMeetingMetadataRequest } from '../../lib/meetings';

interface MeetingMetadataEditorProps {
  session: MeetingSession;
  disabled: boolean;
  onSave: (request: SaveMeetingMetadataRequest) => Promise<boolean>;
  onNotice: (message: string) => void;
}

export function MeetingMetadataEditor({ session, disabled, onSave, onNotice }: MeetingMetadataEditorProps) {
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(session.title ?? '');
  const [attendees, setAttendees] = useState(session.attendees.join('\n'));
  const [saving, setSaving] = useState(false);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const beginEdit = () => {
    setTitle(session.title ?? '');
    setAttendees(session.attendees.join('\n'));
    setEditing(true);
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
      setEditing(false);
      onNotice('Meeting details saved on this Mac.');
    }
  };

  return (
    <section className="dialog-card mb-3 p-3" aria-label="Meeting details">
      {editing ? (
        <form aria-label="Name meeting" aria-busy={saving} onSubmit={(event) => { event.preventDefault(); void save(); }}>
          <fieldset disabled={disabled || saving} className="space-y-2">
            <label className="block text-xs font-semibold text-on-surface">Meeting title
              <input autoFocus value={title} maxLength={200} onChange={(event) => setTitle(event.target.value)} placeholder="Untitled meeting" className="mt-1 w-full rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-low px-2 py-1.5 text-xs focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary" />
            </label>
            <label className="block text-xs font-semibold text-on-surface">Attendees, one per line
              <textarea value={attendees} maxLength={20100} rows={2} onChange={(event) => setAttendees(event.target.value)} className="mt-1 w-full resize-y rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] bg-surface-container-low px-2 py-1.5 text-xs focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary" />
            </label>
            <div className="flex gap-2">
              <button type="submit" className="dialog-pill-btn px-3 py-1.5 text-xs text-primary">{saving ? 'Saving…' : 'Save details'}</button>
              <button type="button" onClick={() => setEditing(false)} className="dialog-pill-btn px-3 py-1.5 text-xs">Cancel naming</button>
            </div>
          </fieldset>
        </form>
      ) : (
        <>
          <div className="flex items-center gap-2">
            <h3 className="min-w-0 flex-1 break-words text-sm font-semibold text-on-surface">{session.title || 'Untitled meeting'}</h3>
            <button type="button" disabled={disabled} onClick={beginEdit} className="dialog-pill-btn shrink-0 px-3 py-1.5 text-xs text-primary disabled:opacity-40">{session.title ? 'Rename' : 'Name meeting'}</button>
          </div>
          {session.attendees.length > 0 && <p className="mt-1 break-words text-xs text-on-surface-variant">Attendees: {session.attendees.join(', ')}</p>}
          {session.titleSource && <p className="mt-1 text-xs text-on-surface-variant">{session.titleSource === 'calendar' ? 'Named from calendar' : session.titleSource === 'generated' ? 'Generated title' : 'Manually named'}</p>}
        </>
      )}
    </section>
  );
}
