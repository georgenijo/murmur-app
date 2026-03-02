import type { StreamName } from '../../lib/events';
import { STREAMS, STREAM_COLORS } from '../../lib/events';

interface StreamChipsProps {
  active: Set<StreamName>;
  onToggle: (stream: StreamName) => void;
}

export function StreamChips({ active, onToggle }: StreamChipsProps) {
  return (
    <div className="flex gap-1.5">
      {STREAMS.map((stream) => {
        const colors = STREAM_COLORS[stream];
        const isActive = active.has(stream);
        return (
          <button
            type="button"
            key={stream}
            onClick={() => onToggle(stream)}
            aria-pressed={isActive}
            className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium transition-all ${
              isActive
                ? `${colors.bg} ${colors.text} ring-1 ring-current/20`
                : 'bg-stone-100 dark:bg-stone-800 text-stone-400 dark:text-stone-500'
            }`}
          >
            <span className={`w-1.5 h-1.5 rounded-full ${isActive ? colors.dot : 'bg-stone-300 dark:bg-stone-600'}`} />
            {stream}
          </button>
        );
      })}
    </div>
  );
}
