import { useId, useMemo } from 'react';
import {
  DAILY_GOAL_WINDOW_DAYS,
  DAILY_WORD_GOAL_OFF,
  DAILY_WORD_GOAL_PRESETS,
  getDailyGoalProgress,
  sanitizeDailyWordGoal,
} from '../../lib/dailyGoal';
import type { DictationStats } from '../../lib/stats';
import { Select, type SelectOption } from '../ui/Select';

interface DailyGoalCardProps {
  stats: DictationStats;
  goal: number;
  onGoalChange: (goal: number) => void;
  now?: Date;
}

function words(value: number): string {
  return `${value.toLocaleString()} ${value === 1 ? 'word' : 'words'}`;
}

export function goalOptions(goal: number): SelectOption[] {
  const values: number[] = [...DAILY_WORD_GOAL_PRESETS];
  // A goal outside the presets (hand-edited settings) stays selectable.
  if (goal !== DAILY_WORD_GOAL_OFF && !values.includes(goal)) values.push(goal);
  values.sort((a, b) => a - b);
  return [
    { value: String(DAILY_WORD_GOAL_OFF), label: 'Off' },
    ...values.map((value) => ({ value: String(value), label: words(value) })),
  ];
}

export function DailyGoalCard({ stats, goal: goalSetting, onGoalChange, now }: DailyGoalCardProps) {
  const titleId = useId();
  const goal = sanitizeDailyWordGoal(goalSetting);
  const progress = useMemo(() => getDailyGoalProgress(stats, goal, now), [stats, goal, now]);
  const state = progress === null ? 'off' : progress.met ? 'met' : 'active';

  return (
    <section className="insights-goal" aria-labelledby={titleId} data-goal-state={state}>
      <div className="insights-goal-header">
        <h2 id={titleId}>Daily word goal</h2>
        <Select
          size="sm"
          value={String(goal)}
          items={goalOptions(goal)}
          onChange={(value) => onGoalChange(sanitizeDailyWordGoal(Number(value)))}
          aria-label="Daily word goal"
        />
      </div>
      {progress === null ? (
        <p className="insights-goal-hint">Pick a target to track today's dictated words here. Dictation itself is unchanged.</p>
      ) : (
        <>
          <div
            className="insights-goal-bar"
            role="progressbar"
            aria-labelledby={titleId}
            aria-valuemin={0}
            aria-valuemax={progress.goal}
            aria-valuenow={Math.min(progress.todayWords, progress.goal)}
            aria-valuetext={`${progress.todayWords.toLocaleString()} of ${words(progress.goal)} today`}
          >
            <span style={{ width: `${progress.percent}%` }} />
          </div>
          <p className="insights-goal-today">
            {progress.met
              ? <>Goal met · {words(progress.todayWords)} today</>
              : <>{progress.todayWords.toLocaleString()} of {words(progress.goal)} today · {progress.remaining.toLocaleString()} to go</>}
          </p>
          <p className="insights-goal-history">
            Met on {progress.metDaysInWindow} of the last {DAILY_GOAL_WINDOW_DAYS} days
            {progress.goalStreak > 1 && <> · {progress.goalStreak}-day goal streak</>}
          </p>
        </>
      )}
    </section>
  );
}
