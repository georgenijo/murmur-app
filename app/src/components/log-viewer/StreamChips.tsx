import type { StreamName } from '../../lib/events';
import { STREAMS, STREAM_COLORS } from '../../lib/events';

interface StreamChipsProps {
  active: Set<StreamName>;
  onToggle: (stream: StreamName) => void;
}

export function StreamChips({ active, onToggle }: StreamChipsProps) {
  return (
    <div className="flex flex-wrap gap-1">
      {STREAMS.map((stream) => {
        const colors = STREAM_COLORS[stream];
        const isActive = active.has(stream);
        return (
          <button
            type="button"
            key={stream}
            onClick={() => onToggle(stream)}
            aria-pressed={isActive}
            className={`inline-flex h-7 items-center gap-1.5 rounded-(--ui-radius-control) px-2 text-[length:var(--ui-font-caption)] font-medium transition-colors ${
              isActive
                ? `${colors.bg} ${colors.text}`
                : 'bg-transparent text-on-surface-variant hover:bg-surface-container'
            }`}
          >
            <span
              aria-hidden="true"
              className={`h-1.5 w-1.5 rounded-full ${
                isActive ? colors.dot : 'bg-on-surface-variant'
              }`}
            />
            {stream}
          </button>
        );
      })}
    </div>
  );
}
