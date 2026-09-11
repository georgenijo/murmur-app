import { useEffect, useRef, useState } from 'react';
import {
  getCalendarPermissionStatus,
  getMeetingCalendarEvents,
  openCalendarPreferences,
  requestCalendarPermission,
  resetCalendarPermission,
  type CalendarEventCandidate,
  type CalendarPermissionState,
} from '../../lib/calendar';

type PickerState =
  | { kind: 'closed' }
  | { kind: 'loading' }
  | { kind: 'unavailable'; message: string; permission: CalendarPermissionState | null }
  | { kind: 'choosing'; events: CalendarEventCandidate[]; selectedToken: string | null }
  | { kind: 'applying'; events: CalendarEventCandidate[]; selectedToken: string };

interface MeetingCalendarNamingProps {
  sessionId: string;
  disabled: boolean;
  onApply: (sessionId: string, selectionToken: string) => Promise<boolean>;
  onManualNaming: () => void;
  onNotice: (message: string) => void;
}

export function MeetingCalendarNaming({ sessionId, disabled, onApply, onManualNaming, onNotice }: MeetingCalendarNamingProps) {
  const [state, setState] = useState<PickerState>({ kind: 'closed' });
  const generation = useRef(0);
  useEffect(() => () => { generation.current += 1; }, []);
  useEffect(() => {
    if (disabled) {
      generation.current += 1;
      setState({ kind: 'closed' });
    }
  }, [disabled]);

  const close = () => {
    generation.current += 1;
    setState({ kind: 'closed' });
  };

  const lookup = async () => {
    if (disabled) return;
    const ticket = ++generation.current;
    setState({ kind: 'loading' });
    try {
      let permission = await getCalendarPermissionStatus();
      if (ticket !== generation.current) return;
      if (permission === 'notDetermined') permission = await requestCalendarPermission();
      if (ticket !== generation.current) return;
      if (permission !== 'granted') {
        setState({ kind: 'unavailable', permission, message: permission === 'unsupported'
          ? 'Calendar lookup is unavailable on this Mac. You can name this meeting manually.'
          : 'Calendar access is not allowed. Enable it in System Settings, or name this meeting manually.' });
        onManualNaming();
        return;
      }
      const events = await getMeetingCalendarEvents(sessionId);
      if (ticket !== generation.current) return;
      setState({ kind: 'choosing', events, selectedToken: null });
    } catch {
      if (ticket !== generation.current) return;
      setState({ kind: 'unavailable', permission: null, message: 'Calendar events could not be read. Try again, or name this meeting manually.' });
      onManualNaming();
    }
  };

  const apply = async () => {
    if (disabled || state.kind !== 'choosing' || !state.selectedToken) return;
    const ticket = ++generation.current;
    setState({ kind: 'applying', events: state.events, selectedToken: state.selectedToken });
    const saved = await onApply(sessionId, state.selectedToken);
    if (ticket !== generation.current) return;
    if (saved) {
      setState({ kind: 'closed' });
      onNotice('Meeting named from calendar.');
    } else {
      setState({ kind: 'unavailable', permission: null, message: 'The event may have changed. Look it up again, or name this meeting manually.' });
      onManualNaming();
    }
  };

  const openSettings = async () => {
    try { await openCalendarPreferences(); } catch { onNotice('Calendar settings could not be opened. Open System Settings, then Privacy & Security, then Calendars.'); }
  };

  const reset = async () => {
    const ticket = ++generation.current;
    setState({ kind: 'loading' });
    try {
      await resetCalendarPermission();
      if (ticket !== generation.current) return;
      setState({ kind: 'closed' });
      onNotice('Calendar access reset. Choose Name from calendar to request access again.');
    } catch {
      if (ticket !== generation.current) return;
      setState({ kind: 'unavailable', permission: 'denied', message: 'Calendar access could not be reset. Check System Settings, or keep naming meetings manually.' });
    }
  };

  const picking = state.kind === 'choosing' || state.kind === 'applying';
  return (
    <div className="mt-2 text-xs">
      <button type="button" disabled={disabled || state.kind === 'loading' || state.kind === 'applying'} onClick={() => void lookup()} className="dialog-pill-btn px-3 py-1.5 text-primary disabled:opacity-40">{state.kind === 'loading' ? 'Reading calendar…' : 'Name from calendar'}</button>
      {state.kind === 'unavailable' && (
        <div role="status" className="mt-2 space-y-2 text-on-surface-variant">
          <p>{state.message}</p>
          <div className="flex flex-wrap gap-2">
            <button type="button" onClick={onManualNaming} className="dialog-pill-btn px-3 py-1.5 text-primary">Name manually</button>
            {state.permission !== 'unsupported' && <button type="button" onClick={() => void openSettings()} className="dialog-pill-btn px-3 py-1.5">Open Calendar Settings</button>}
            {state.permission === 'denied' && <button type="button" onClick={() => void reset()} className="dialog-pill-btn px-3 py-1.5">Reset Calendar Access</button>}
          </div>
        </div>
      )}
      {picking && (
        <fieldset disabled={disabled || state.kind === 'applying'} aria-busy={state.kind === 'applying'} className="mt-2 space-y-2">
          <legend className="font-semibold text-on-surface">Choose an overlapping event</legend>
          {state.events.length === 0 ? <p className="text-on-surface-variant">No calendar events overlap this meeting. You can still name it manually.</p> : (
            <>
              <p className="text-on-surface-variant">Only the title and attendee names of the event you apply will be saved.</p>
              <div className="max-h-64 space-y-2 overflow-y-auto">
                {state.events.map((event) => (
                  <label key={event.selectionToken} className="flex items-start gap-2 rounded-[var(--ui-radius-control)] border border-[var(--ui-hairline)] p-2">
                    <input type="radio" name={`calendar-event-${sessionId}`} checked={state.selectedToken === event.selectionToken} onChange={() => setState({ kind: 'choosing', events: state.events, selectedToken: event.selectionToken })} />
                    <span className="min-w-0 break-words">
                      <span className="block font-semibold text-on-surface">{event.title}</span>
                      <span className="block text-on-surface-variant">{new Date(event.startMs).toLocaleString()} – {new Date(event.endMs).toLocaleTimeString()}</span>
                      {event.attendees.length > 0 && <span className="block text-on-surface-variant">Attendees: {event.attendees.join(', ')}</span>}
                    </span>
                  </label>
                ))}
              </div>
            </>
          )}
          <div className="flex gap-2">
            <button type="button" disabled={!state.selectedToken} onClick={() => void apply()} className="dialog-pill-btn px-3 py-1.5 text-primary disabled:opacity-40">{state.kind === 'applying' ? 'Applying…' : 'Apply event'}</button>
            <button type="button" onClick={close} className="dialog-pill-btn px-3 py-1.5">Cancel calendar lookup</button>
          </div>
        </fieldset>
      )}
    </div>
  );
}
