import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CalendarEventCandidate } from '../../lib/calendar';
import { MeetingCalendarNaming } from './MeetingCalendarNaming';

const calendar = vi.hoisted(() => ({
  getCalendarPermissionStatus: vi.fn(),
  requestCalendarPermission: vi.fn(),
  getMeetingCalendarEvents: vi.fn(),
  openCalendarPreferences: vi.fn(),
  resetCalendarPermission: vi.fn(),
}));
vi.mock('../../lib/calendar', () => calendar);

const event: CalendarEventCandidate = {
  selectionToken: 'selection-one', title: 'Planning session', attendees: ['Alex Example'],
  startMs: 1000, endMs: 2000,
};

describe('MeetingCalendarNaming', () => {
  let root: Root;
  let container: HTMLDivElement;
  const onApply = vi.fn();
  const onManualNaming = vi.fn();
  const onNotice = vi.fn();
  const render = async (sessionId = 'meeting-one', disabled = false) => {
    await act(async () => root.render(<MeetingCalendarNaming key={sessionId} sessionId={sessionId} disabled={disabled} onApply={onApply} onManualNaming={onManualNaming} onNotice={onNotice} />));
  };
  const button = (label: string) => {
    const found = [...container.querySelectorAll('button')].find((candidate) => candidate.textContent === label);
    if (!found) throw new Error(`Missing button: ${label}`);
    return found;
  };
  const click = async (label: string) => { await act(async () => button(label).click()); };

  beforeEach(() => {
    vi.resetAllMocks();
    calendar.getCalendarPermissionStatus.mockResolvedValue('granted');
    calendar.requestCalendarPermission.mockResolvedValue('granted');
    calendar.getMeetingCalendarEvents.mockResolvedValue([event]);
    onApply.mockResolvedValue(true);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

  it.each([1, 2])('reads only after the action and requires explicit selection for %i events', async (count) => {
    calendar.getMeetingCalendarEvents.mockResolvedValue(Array.from({ length: count }, (_, index) => ({ ...event, selectionToken: `selection-${index}`, title: `Planning ${index}` })));
    await render();
    expect(calendar.getCalendarPermissionStatus).not.toHaveBeenCalled();
    expect(calendar.getMeetingCalendarEvents).not.toHaveBeenCalled();
    await click('Name from calendar');
    expect(calendar.getMeetingCalendarEvents).toHaveBeenCalledWith('meeting-one');
    expect(calendar.requestCalendarPermission).not.toHaveBeenCalled();
    expect(container.querySelectorAll('input[type="radio"]')).toHaveLength(count);
    expect(container.querySelector('input:checked')).toBeNull();
    expect(button('Apply event').disabled).toBe(true);
    expect(onApply).not.toHaveBeenCalled();
    const radio = container.querySelector<HTMLInputElement>('input[type="radio"]');
    if (!radio) throw new Error('Missing event option');
    await act(async () => radio.click());
    await click('Apply event');
    expect(onApply).toHaveBeenCalledWith('meeting-one', 'selection-0');
    expect(container.querySelector('input[type="radio"]')).toBeNull();
  });

  it('requests new access only after a click and falls back to manual naming on denial', async () => {
    calendar.getCalendarPermissionStatus.mockResolvedValue('notDetermined');
    calendar.requestCalendarPermission.mockResolvedValue('denied');
    await render();
    expect(calendar.requestCalendarPermission).not.toHaveBeenCalled();
    await click('Name from calendar');
    expect(calendar.requestCalendarPermission).toHaveBeenCalledOnce();
    expect(calendar.getMeetingCalendarEvents).not.toHaveBeenCalled();
    expect(onManualNaming).toHaveBeenCalledOnce();
    expect(container.textContent).toContain('Calendar access is not allowed');
    await click('Open Calendar Settings');
    expect(calendar.openCalendarPreferences).toHaveBeenCalledOnce();
    await click('Reset Calendar Access');
    expect(calendar.resetCalendarPermission).toHaveBeenCalledOnce();
    expect(calendar.requestCalendarPermission).toHaveBeenCalledOnce();
  });

  it('discards private errors and exposes manual naming on lookup failure', async () => {
    calendar.getMeetingCalendarEvents.mockRejectedValue(new Error('PRIVATE_CALENDAR_SENTINEL'));
    await render();
    await click('Name from calendar');
    expect(container.textContent).toContain('Calendar events could not be read');
    expect(container.textContent).not.toContain('PRIVATE_CALENDAR_SENTINEL');
    expect(onManualNaming).toHaveBeenCalledOnce();
    expect(onApply).not.toHaveBeenCalled();
  });

  it('drops a late lookup after selecting another meeting', async () => {
    let resolve: (events: CalendarEventCandidate[]) => void = () => {};
    calendar.getMeetingCalendarEvents.mockReturnValue(new Promise<CalendarEventCandidate[]>((done) => { resolve = done; }));
    await render();
    await click('Name from calendar');
    await render('meeting-two');
    await act(async () => resolve([event]));
    expect(container.textContent).not.toContain(event.title);
    expect(onApply).not.toHaveBeenCalled();
  });

  it('does not read events after capture becomes busy during a permission check', async () => {
    let resolve: (permission: string) => void = () => {};
    calendar.getCalendarPermissionStatus.mockReturnValue(new Promise<string>((done) => { resolve = done; }));
    await render();
    await click('Name from calendar');
    await render('meeting-one', true);
    await act(async () => resolve('granted'));
    expect(calendar.getMeetingCalendarEvents).not.toHaveBeenCalled();
    expect(button('Name from calendar').disabled).toBe(true);
  });

  it('discards the candidate list when the selected event changes before apply', async () => {
    onApply.mockResolvedValue(false);
    await render();
    await click('Name from calendar');
    const radio = container.querySelector<HTMLInputElement>('input[type="radio"]');
    if (!radio) throw new Error('Missing event');
    await act(async () => radio.click());
    await click('Apply event');
    expect(container.textContent).toContain('The event may have changed');
    expect(container.textContent).not.toContain(event.title);
    expect(onManualNaming).toHaveBeenCalledOnce();
  });
});
