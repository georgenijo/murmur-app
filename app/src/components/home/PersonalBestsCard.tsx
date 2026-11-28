import { useId, useMemo } from 'react';
import { dateForKey, getPersonalBests, hasPersonalBests } from '../../lib/personalBests';
import type { DictationStats } from '../../lib/stats';
import AnimatedSwitch from '../ui/animated-switch/animated-switch';

interface PersonalBestsCardProps {
  stats: DictationStats;
  enabled: boolean;
  onEnabledChange: (enabled: boolean) => void;
}

const DAY_FORMAT: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric', year: 'numeric' };

function plural(value: number, one: string, many: string): string {
  return `${value.toLocaleString()} ${value === 1 ? one : many}`;
}

export function formatBestDay(key: string): string {
  return dateForKey(key)?.toLocaleDateString(undefined, DAY_FORMAT) ?? key;
}

export function formatBestRange(startKey: string, endKey: string): string {
  if (startKey === endKey) return formatBestDay(startKey);
  return `${formatBestDay(startKey)} – ${formatBestDay(endKey)}`;
}

export function PersonalBestsCard({ stats, enabled, onEnabledChange }: PersonalBestsCardProps) {
  const titleId = useId();
  const bests = useMemo(() => getPersonalBests(stats), [stats]);
  const state = !enabled ? 'off' : hasPersonalBests(bests) ? 'on' : 'empty';

  return (
    <section className="insights-bests" aria-labelledby={titleId} data-bests-state={state}>
      <div className="insights-bests-header">
        <h2 id={titleId}>Personal bests</h2>
        <AnimatedSwitch size="sm" aria-label="Show personal bests" checked={enabled} onCheckedChange={onEnabledChange} />
      </div>
      {state === 'off' && (
        <p className="insights-bests-hint">Turn on to see your best day, week and streak from local statistics. Dictation itself is unchanged.</p>
      )}
      {state === 'empty' && (
        <p className="insights-bests-hint">Your bests appear here after your first dictation.</p>
      )}
      {state === 'on' && (
        <dl className="insights-bests-grid" aria-label="Personal bests">
          {bests.mostWordsDay && (
            <div data-best="words-day">
              <dt>Most words in a day</dt>
              <dd><strong>{plural(bests.mostWordsDay.value, 'word', 'words')}</strong><small>{formatBestDay(bests.mostWordsDay.key)}</small></dd>
            </div>
          )}
          {bests.bestWeek && (
            <div data-best="week">
              <dt>Best week</dt>
              <dd><strong>{plural(bests.bestWeek.words, 'word', 'words')}</strong><small>Week of {formatBestDay(bests.bestWeek.startKey)}</small></dd>
            </div>
          )}
          {bests.mostDictationsDay && (
            <div data-best="dictations-day">
              <dt>Most dictations in a day</dt>
              <dd><strong>{plural(bests.mostDictationsDay.value, 'dictation', 'dictations')}</strong><small>{formatBestDay(bests.mostDictationsDay.key)}</small></dd>
            </div>
          )}
          {bests.longestStreak && (
            <div data-best="streak">
              <dt>Longest streak</dt>
              <dd><strong>{plural(bests.longestStreak.days, 'day', 'days')}</strong><small>{formatBestRange(bests.longestStreak.startKey, bests.longestStreak.endKey)}</small></dd>
            </div>
          )}
        </dl>
      )}
    </section>
  );
}
