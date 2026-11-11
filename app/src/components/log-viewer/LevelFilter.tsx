import type { LevelName } from '../../lib/events';

const FILTER_LEVELS: LevelName[] = ['info', 'warn', 'error'];

interface LevelFilterProps {
  active: Set<LevelName>;
  onToggle: (level: LevelName) => void;
}

export function LevelFilter({ active, onToggle }: LevelFilterProps) {
  return (
    <div className="settings-segmented h-7" role="group" aria-label="Event levels">
      {FILTER_LEVELS.map((level) => {
        const isActive = active.has(level);
        return (
          <button
            type="button"
            key={level}
            onClick={() => onToggle(level)}
            aria-pressed={isActive}
            data-selected={isActive}
            className="settings-segmented-btn !min-w-0 !px-2"
          >
            {level}
          </button>
        );
      })}
    </div>
  );
}
