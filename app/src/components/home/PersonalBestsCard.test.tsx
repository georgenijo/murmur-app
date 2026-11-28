import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { formatBestDay, formatBestRange, PersonalBestsCard } from './PersonalBestsCard';
import { InsightsView } from './InsightsView';
import { loadStats, type DictationStats } from '../../lib/stats';

let container: HTMLDivElement;
let root: Root;

const populated: DictationStats = {
  ...loadStats(),
  dailyBuckets: {
    '2026-11-01': { words: 300, recordings: 2, recordingSeconds: 60 },
    '2026-11-02': { words: 900, recordings: 7, recordingSeconds: 300 },
    '2026-11-03': { words: 1, recordings: 1, recordingSeconds: 5 },
  },
};

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

describe('PersonalBestsCard', () => {
  it('shows only the switch and an explanation while off', async () => {
    await render(<PersonalBestsCard stats={populated} enabled={false} onEnabledChange={() => {}} />);
    const card = container.querySelector('.insights-bests')!;
    expect(card.getAttribute('data-bests-state')).toBe('off');
    expect(card.querySelector('dl')).toBeNull();
    expect(card.textContent).toContain('Dictation itself is unchanged.');
    expect(container.querySelector('[aria-label="Show personal bests"]')?.getAttribute('aria-checked')).toBe('false');
  });

  it('lists the four bests with dates when on', async () => {
    await render(<PersonalBestsCard stats={populated} enabled onEnabledChange={() => {}} />);
    const card = container.querySelector('.insights-bests')!;
    expect(card.getAttribute('data-bests-state')).toBe('on');
    const text = (best: string) => card.querySelector(`[data-best="${best}"] dd`)?.textContent;
    expect(text('words-day')).toBe(`900 words${formatBestDay('2026-11-02')}`);
    expect(text('week')).toBe(`1,201 wordsWeek of ${formatBestDay('2026-11-01')}`);
    expect(text('dictations-day')).toBe(`7 dictations${formatBestDay('2026-11-02')}`);
    expect(text('streak')).toBe(`3 days${formatBestRange('2026-11-01', '2026-11-03')}`);
  });

  it('uses singular units and a single date for a one-day streak', async () => {
    const one: DictationStats = { ...loadStats(), dailyBuckets: { '2026-11-03': { words: 1, recordings: 1, recordingSeconds: 5 } } };
    await render(<PersonalBestsCard stats={one} enabled onEnabledChange={() => {}} />);
    expect(container.querySelector('[data-best="words-day"] strong')?.textContent).toBe('1 word');
    expect(container.querySelector('[data-best="dictations-day"] strong')?.textContent).toBe('1 dictation');
    expect(container.querySelector('[data-best="streak"] strong')?.textContent).toBe('1 day');
    expect(container.querySelector('[data-best="streak"] small')?.textContent).toBe(formatBestDay('2026-11-03'));
  });

  it('omits word bests when no words were transcribed', async () => {
    const silent: DictationStats = { ...loadStats(), dailyBuckets: { '2026-11-03': { words: 0, recordings: 2, recordingSeconds: 5 } } };
    await render(<PersonalBestsCard stats={silent} enabled onEnabledChange={() => {}} />);
    expect(container.querySelector('[data-best="words-day"]')).toBeNull();
    expect(container.querySelector('[data-best="week"]')).toBeNull();
    expect(container.querySelector('[data-best="dictations-day"]')).not.toBeNull();
  });

  it('invites a first dictation when on with no statistics', async () => {
    await render(<PersonalBestsCard stats={{ ...loadStats(), dailyBuckets: {} }} enabled onEnabledChange={() => {}} />);
    expect(container.querySelector('.insights-bests')?.getAttribute('data-bests-state')).toBe('empty');
    expect(container.querySelector('.insights-bests-hint')?.textContent).toBe('Your bests appear here after your first dictation.');
  });

  it('reports the switch through the change handler', async () => {
    const onEnabledChange = vi.fn();
    await render(<PersonalBestsCard stats={populated} enabled={false} onEnabledChange={onEnabledChange} />);
    await act(async () => container.querySelector<HTMLElement>('[aria-label="Show personal bests"]')!.click());
    expect(onEnabledChange).toHaveBeenCalledWith(true);
  });
});

describe('InsightsView personal bests', () => {
  it('omits the card without a change handler and shows it with one', async () => {
    await render(<InsightsView statsVersion={0} onBackToHome={() => {}} />);
    expect(container.querySelector('.insights-bests')).toBeNull();
    await render(<InsightsView statsVersion={0} onShowPersonalBestsChange={() => {}} onBackToHome={() => {}} />);
    expect(container.querySelector('.insights-bests')?.getAttribute('data-bests-state')).toBe('off');
  });
});
