import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DailyGoalCard, goalOptions } from './DailyGoalCard';
import { InsightsView } from './InsightsView';
import { dayKey, loadStats, type DictationStats } from '../../lib/stats';

const NOW = new Date(2026, 10, 27, 15, 0, 0);
let container: HTMLDivElement;
let root: Root;

function stats(todayWords: number, yesterdayWords = 0): DictationStats {
  const yesterday = new Date(NOW.getFullYear(), NOW.getMonth(), NOW.getDate() - 1);
  return {
    ...loadStats(),
    dailyBuckets: {
      [dayKey(NOW)]: { words: todayWords, recordings: 1, recordingSeconds: 60 },
      [dayKey(yesterday)]: { words: yesterdayWords, recordings: 1, recordingSeconds: 60 },
    },
  };
}

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

describe('DailyGoalCard', () => {
  it('shows only the picker and an explanation while off', async () => {
    await render(<DailyGoalCard stats={stats(300)} goal={0} onGoalChange={() => {}} now={NOW} />);
    const card = container.querySelector('.insights-goal')!;
    expect(card.getAttribute('data-goal-state')).toBe('off');
    expect(card.querySelector('[role="progressbar"]')).toBeNull();
    expect(card.textContent).toContain('Dictation itself is unchanged.');
    expect(container.querySelector('[aria-label="Daily word goal"]')?.textContent).toContain('Off');
  });

  it('exposes accessible progress toward an active goal', async () => {
    await render(<DailyGoalCard stats={stats(312, 600)} goal={500} onGoalChange={() => {}} now={NOW} />);
    const bar = container.querySelector('[role="progressbar"]')!;
    expect(bar.getAttribute('aria-valuenow')).toBe('312');
    expect(bar.getAttribute('aria-valuemax')).toBe('500');
    expect(bar.getAttribute('aria-valuetext')).toBe('312 of 500 words today');
    expect((bar.firstElementChild as HTMLElement).style.width).toBe('62%');
    expect(container.querySelector('.insights-goal-today')?.textContent).toBe('312 of 500 words today · 188 to go');
    expect(container.querySelector('.insights-goal-history')?.textContent).toBe('Met on 1 of the last 7 days');
  });

  it('marks a met goal, clamps aria-valuenow, and names the streak', async () => {
    await render(<DailyGoalCard stats={stats(760, 600)} goal={500} onGoalChange={() => {}} now={NOW} />);
    expect(container.querySelector('.insights-goal')?.getAttribute('data-goal-state')).toBe('met');
    expect(container.querySelector('[role="progressbar"]')?.getAttribute('aria-valuenow')).toBe('500');
    expect(container.querySelector('.insights-goal-today')?.textContent).toBe('Goal met · 760 words today');
    expect(container.querySelector('.insights-goal-history')?.textContent).toBe('Met on 2 of the last 7 days · 2-day goal streak');
  });

  it('reports the chosen preset through the change handler', async () => {
    const onGoalChange = vi.fn();
    await render(<DailyGoalCard stats={stats(0)} goal={0} onGoalChange={onGoalChange} now={NOW} />);
    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="Daily word goal"]')!;
    await act(async () => trigger.click());
    const option = Array.from(document.querySelectorAll<HTMLElement>('[role="option"]')).find((el) => el.textContent?.includes('1,000 words'))!;
    await act(async () => option.click());
    expect(onGoalChange).toHaveBeenCalledWith(1000);
  });

  it('keeps a hand-edited goal selectable in order', () => {
    expect(goalOptions(750).map((option) => option.value)).toEqual(['0', '250', '500', '750', '1000', '2000', '5000']);
    expect(goalOptions(0)).toHaveLength(6);
  });
});

describe('InsightsView daily goal', () => {
  it('omits the card without a change handler and shows it with one', async () => {
    await render(<InsightsView statsVersion={0} onBackToHome={() => {}} />);
    expect(container.querySelector('.insights-goal')).toBeNull();
    await render(<InsightsView statsVersion={0} dailyWordGoal={0} onDailyWordGoalChange={() => {}} onBackToHome={() => {}} />);
    expect(container.querySelector('.insights-goal')?.getAttribute('data-goal-state')).toBe('off');
  });
});
