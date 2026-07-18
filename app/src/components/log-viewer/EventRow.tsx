import { useState } from 'react';
import type { AppEvent } from '../../lib/events';
import { STREAM_COLORS, LEVEL_COLORS } from '../../lib/events';
import type { StreamName, LevelName } from '../../lib/events';

interface EventRowProps {
  event: AppEvent;
}

export function EventRow({ event }: EventRowProps) {
  const [expanded, setExpanded] = useState(false);
  const streamColors = STREAM_COLORS[event.stream as StreamName] ?? STREAM_COLORS.system;
  const levelColor = LEVEL_COLORS[event.level as LevelName] ?? LEVEL_COLORS.info;
  const hasData = event.data && typeof event.data === 'object' && Object.keys(event.data).length > 0;

  // Format timestamp to show just time portion
  const timeStr = event.timestamp.replace(/.*T/, '').replace('Z', '');

  return (
    <div>
      <div
        className={`flex items-baseline gap-2 py-1 px-1 ${hasData ? 'cursor-pointer hover:bg-surface-container-low' : ''}`}
        onClick={() => hasData && setExpanded(!expanded)}
        {...(hasData && {
          role: 'button',
          tabIndex: 0,
          'aria-expanded': expanded,
          onKeyDown: (e: React.KeyboardEvent) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              setExpanded(s => !s);
            }
          },
        })}
      >
        <span className="text-on-surface-variant shrink-0 tabular-nums text-[11px]">
          {timeStr}
        </span>
        <span className={`inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-semibold shrink-0 ${streamColors.bg} ${streamColors.text}`}>
          {event.stream}
        </span>
        <span className={`text-[10px] font-medium shrink-0 uppercase ${levelColor}`}>
          {event.level}
        </span>
        <span className="text-on-surface text-xs break-all flex-1">
          {event.summary}
        </span>
        {hasData && (
          <span className={`text-on-surface-variant text-xs shrink-0 transition-transform ${expanded ? 'rotate-90' : ''}`}>
            &#9656;
          </span>
        )}
      </div>
      {expanded && hasData && (
        <pre className="mx-1 mb-1 px-3 py-2 bg-surface-container-low rounded text-[11px] text-on-surface-variant overflow-x-auto">
          {JSON.stringify(event.data, null, 2)}
        </pre>
      )}
    </div>
  );
}
