import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DictationHoursCard } from './DictationHoursCard';
import { InsightsView } from './InsightsView';
import type { HistoryEntry } from '../../lib/history';

let container: HTMLDivElement;
let root: Root;

function at(hour: number, index: number, extra: Partial<HistoryEntry> = {}): HistoryEntry {
  return { id: `h-${hour}-${index}`, text: 'private words', timestamp: new Date(2026, 10, 20, hour, index).getTime(), duration: 4, ...extra } as HistoryEntry;
}

const entries: HistoryEntry[] = [at(9, 0), at(9, 1), at(9, 2), at(15, 0), at(20, 0), at(23, 0), at(8, 0, { source: 'file', sourceName: 'a.wav' })];

async function render(node: React.ReactNode) {
  await act(async () => root.render(node));
}

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe('DictationHoursCard', () => {
  it('shows only the switch and an explanation while off', async () => {
    await render(<DictationHoursCard entries={entries} enabled={false} onEnabledChange={() => {}} />);
    const card = container.querySelector('.insights-hours')!;
    expect(card.getAttribute('data-hours-state')).toBe('off');
    expect(card.querySelector('.insights-hours-bars')).toBeNull();
    expect(card.querySelector('[role="switch"]')?.getAttribute('aria-checked')).toBe('false');
    expect(card.textContent).toContain('Dictation itself is unchanged');
  });

  it('invites a first dictation when History has no spoken entries', async () => {
    await render(<DictationHoursCard entries={[entries[6]]} enabled onEnabledChange={() => {}} />);
    const card = container.querySelector('.insights-hours')!;
    expect(card.getAttribute('data-hours-state')).toBe('empty');
    expect(card.textContent).toContain('first spoken dictation');
  });

  it('plots 24 hours, marks the peak, and never renders transcript text', async () => {
    await render(<DictationHoursCard entries={entries} enabled onEnabledChange={() => {}} />);
    const card = container.querySelector('.insights-hours')!;
    expect(card.getAttribute('data-hours-state')).toBe('on');
    const bars = card.querySelectorAll('.insights-hours-bars > span');
    expect(bars).toHaveLength(24);
    expect(card.querySelectorAll('[data-peak]')).toHaveLength(1);
    expect(card.querySelector('[data-peak]')?.getAttribute('data-hour')).toBe('9');
    expect((bars[9] as HTMLElement).style.height).toBe('100%');
    expect((bars[8] as HTMLElement).style.height).toBe('');
    expect(card.querySelector('.insights-hours-summary')?.textContent).toContain('From 6 dictations in History');
    expect(card.querySelector('[role="img"]')?.getAttribute('aria-label')).toContain('3 dictations');
    expect(card.querySelectorAll('table tbody tr')).toHaveLength(24);
    const shares = Object.fromEntries(Array.from(card.querySelectorAll('.insights-hours-periods > div')).map((cell) => [
      cell.getAttribute('data-period'), cell.querySelector('dd')?.textContent,
    ]));
    expect(shares).toEqual({ morning: '50%', afternoon: '17%', evening: '17%', night: '17%' });
    expect(card.textContent).not.toContain('private words');
  });

  it('reports the switch change', async () => {
    const onEnabledChange = vi.fn();
    await render(<DictationHoursCard entries={entries} enabled={false} onEnabledChange={onEnabledChange} />);
    await act(async () => (container.querySelector('[role="switch"]') as HTMLButtonElement).click());
    expect(onEnabledChange).toHaveBeenCalledWith(true);
  });
});

describe('InsightsView time-of-day card', () => {
  it('renders the card only when a change handler is wired', async () => {
    await render(<InsightsView statsVersion={0} onBackToHome={() => {}} historyEntries={entries} />);
    expect(container.querySelector('.insights-hours')).toBeNull();
    await render(<InsightsView statsVersion={0} onBackToHome={() => {}} historyEntries={entries} showDictationHours onShowDictationHoursChange={() => {}} />);
    expect(container.querySelector('.insights-hours')?.getAttribute('data-hours-state')).toBe('on');
  });
});
