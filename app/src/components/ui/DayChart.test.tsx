import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { DaySummary } from '../../lib/stats';
import { DayChart } from './DayChart';

function day(index: number, overrides: Partial<DaySummary> = {}): DaySummary {
  const date = new Date(2026, 7, 2 + index);
  return {
    key: `2026-08-${String(2 + index).padStart(2, '0')}`,
    date,
    words: index * 100,
    recordings: index,
    recordingSeconds: index * 30,
    wpm: index === 0 ? 0 : 160 + index,
    ...overrides,
  };
}

describe('DayChart', () => {
  let container: HTMLDivElement;
  let root: Root;
  const days = Array.from({ length: 7 }, (_, index) => day(index));

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('keeps bars, tooltip, plot, and seven labels in separate regions', async () => {
    await act(async () => root.render(
      <DayChart kind="bars" metric="words" days={days} ariaLabel="Weekly words" highlightLast />,
    ));

    expect(container.querySelectorAll('.ui-day-chart-bar')).toHaveLength(7);
    expect(container.querySelectorAll('.ui-day-chart-axis span')).toHaveLength(7);
    const last = container.querySelectorAll('.ui-day-chart-bar')[6] as HTMLButtonElement;
    expect(last.dataset.highlighted).toBe('true');
    await act(async () => last.click());
    expect(container.querySelector('.ui-day-chart-tooltip')?.textContent).toContain('600 words');
    expect(last.getAttribute('aria-label')).toContain('6 recordings');
  });

  it('renders a keyboard target and explicit no-data label for every WPM day', async () => {
    await act(async () => root.render(
      <DayChart kind="line" metric="wpm" days={days} ariaLabel="WPM trend" />,
    ));

    const targets = container.querySelectorAll('.ui-day-chart-line-targets button');
    expect(targets).toHaveLength(7);
    expect(targets[0].getAttribute('aria-label')).toContain('No WPM data');
    await act(async () => (targets[1] as HTMLButtonElement).focus());
    expect(container.querySelector('.ui-day-chart-tooltip')?.textContent).toContain('161 WPM');
    await act(async () => targets[1].dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
    expect(container.querySelector('.ui-day-chart-tooltip')?.textContent).toContain('Focus a day');
  });

  it('invites dictation when the Speed chart has no recordings', async () => {
    const emptyDays = days.map(value => ({ ...value, recordings: 0, wpm: 0 }));
    await act(async () => root.render(
      <DayChart kind="line" metric="wpm" days={emptyDays} ariaLabel="WPM trend" presentation="insights" />,
    ));
    expect(container.querySelector('.insights-empty-message')?.textContent)
      .toBe('Dictate something and your activity will show up here.');
    expect(container.querySelectorAll('.ui-day-chart-plot button, .ui-day-chart-plot svg')).toHaveLength(0);
    expect(container.querySelectorAll('.ui-day-chart-axis span')).toHaveLength(7);
  });

  it('explains missing speed while preserving all seven accessible day targets', async () => {
    const noSpeedDays = days.map((value, index) => ({ ...value, recordings: 1, wpm: index === 0 ? NaN : 0 }));
    await act(async () => root.render(
      <DayChart kind="line" metric="wpm" days={noSpeedDays} ariaLabel="WPM trend" presentation="insights" />,
    ));
    expect(container.querySelector('.insights-empty-message')?.textContent).toBe('No speed data for these days.');
    expect(container.querySelector('figure')?.getAttribute('aria-label')).toContain('no speed data');
    const targets = container.querySelectorAll('.ui-day-chart-line-targets button');
    expect(targets).toHaveLength(7);
    expect([...targets].every(target => target.getAttribute('aria-label')?.includes('No WPM data'))).toBe(true);
    expect(container.querySelectorAll('polyline, .ui-day-chart-line-targets span')).toHaveLength(0);
    await act(async () => (targets[0] as HTMLButtonElement).focus());
    expect(container.querySelector('[role="status"]')?.textContent).toContain('No WPM data');
    expect(container.querySelector('[role="status"]')?.textContent).toContain('1 recording');
  });

  it('plots usable speed without an empty-state explanation', async () => {
    await act(async () => root.render(
      <DayChart kind="line" metric="wpm" days={days} ariaLabel="WPM trend" presentation="insights" />,
    ));
    expect(container.querySelector('.insights-empty-message')).toBeNull();
    expect(container.querySelectorAll('.ui-day-chart-line-targets button')).toHaveLength(7);
    expect(container.querySelectorAll('.ui-day-chart-line-targets span')).toHaveLength(6);
    expect(container.querySelectorAll('polyline')).toHaveLength(1);
    expect(container.querySelector('figure')?.getAttribute('aria-label')).toContain('speed axis');
  });

  it('keeps both years visible across a calendar rollover and tolerates empty weeks', async () => {
    const weeks = [[day(0, { date: new Date(2026, 11, 27) })], [day(1, { date: new Date(2027, 0, 3) })]];
    await act(async () => root.render(
      <DayChart kind="heatmap" metric="words" weeks={weeks} ariaLabel="Activity heatmap" />,
    ));
    const range = container.querySelector('.ui-day-chart-date-range')?.textContent;
    expect(range).toContain('2026');
    expect(range).toContain('2027');
    expect([...container.querySelectorAll('.ui-day-chart-months span')].map((label) => label.textContent))
      .toEqual(['Dec', 'Jan']);
    await act(async () => root.render(
      <DayChart kind="heatmap" metric="words" weeks={[]} ariaLabel="Activity heatmap" />,
    ));
    expect(container.querySelector('.ui-day-chart-date-range')).toBeNull();
    expect(container.querySelectorAll('.ui-day-chart-heatmap button')).toHaveLength(0);
  });

  it('makes every heatmap cell focusable with bounded intensity and exact values', async () => {
    const weeks = Array.from({ length: 8 }, (_, week) => (
      Array.from({ length: 7 }, (_, weekday) => day(week * 7 + weekday, {
        key: `day-${week}-${weekday}`,
        date: new Date(2026, 6, 5 + week * 7 + weekday),
      }))
    ));
    await act(async () => root.render(
      <DayChart kind="heatmap" metric="words" weeks={weeks} ariaLabel="Activity heatmap" />,
    ));

    const cells = container.querySelectorAll('.ui-day-chart-heatmap button');
    expect(cells).toHaveLength(56);
    expect(container.querySelector('.ui-day-chart-date-range')?.textContent).toContain('2026');
    expect([...container.querySelectorAll('.ui-day-chart-months span')].map((label) => label.textContent))
      .toEqual(['Jul', '', '', '', 'Aug', '', '', '']);
    expect([...container.querySelectorAll('.ui-day-chart-weekdays span')].map((label) => label.textContent))
      .toEqual(['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']);
    expect(cells[10].getAttribute('aria-label')).toBe('Jul 15, 2026, 1,000 words, 10 recordings');
    const table = container.querySelector('.ui-day-chart-values table');
    expect(table?.querySelectorAll('tbody tr')).toHaveLength(56);
    expect(table?.querySelectorAll('tbody tr')[10].textContent).toBe('Jul 15, 20261,00010');
    expect(container.querySelector('.ui-day-chart-values summary')?.textContent).toBe('Daily values');
    expect(Array.from(cells).every((cell) => /^[0-4]$/.test((cell as HTMLElement).dataset.intensity ?? ''))).toBe(true);
    await act(async () => (cells[10] as HTMLButtonElement).click());
    const tooltip = container.querySelector('.ui-day-chart-tooltip')?.textContent ?? '';
    expect(tooltip).toContain('Jul 15, 2026');
    expect(tooltip).toContain('1,000 words');
    expect(tooltip).toContain('10 recordings');
    await act(async () => document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })));
    expect(container.querySelector('.ui-day-chart-tooltip')?.textContent).toContain('Focus a day');
  });
});
