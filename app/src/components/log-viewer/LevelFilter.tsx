import type { LevelName } from '../../lib/events';

const FILTER_LEVELS: LevelName[] = ['info', 'warn', 'error'];

interface LevelFilterProps {
  active: Set<LevelName>;
  onToggle: (level: LevelName) => void;
}

export function LevelFilter({ active, onToggle }: LevelFilterProps) {
  return (
    <div className="flex gap-1">
      {FILTER_LEVELS.map((level) => {
        const isActive = active.has(level);
        return (
          <button
            type="button"
            key={level}
            onClick={() => onToggle(level)}
            aria-pressed={isActive}
            className={`px-2 py-0.5 rounded text-xs font-medium transition-colors ${
              isActive
                ? 'bg-stone-800 dark:bg-stone-200 text-white dark:text-stone-900'
                : 'bg-stone-100 dark:bg-stone-800 text-stone-400 dark:text-stone-500 hover:bg-stone-200 dark:hover:bg-stone-700'
            }`}
          >
            {level}
          </button>
        );
      })}
    </div>
  );
}
