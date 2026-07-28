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
                ? 'bg-primary text-on-primary'
                : 'bg-surface-container text-on-surface-variant hover:bg-surface-container-high'
            }`}
          >
            {level}
          </button>
        );
      })}
    </div>
  );
}
