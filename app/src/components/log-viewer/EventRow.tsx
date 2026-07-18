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
    <div className={event.level === 'error' ? 'bg-error/10' : ''}>
      <div
        className={`grid grid-cols-[88px_52px_88px_minmax(0,1fr)] items-baseline gap-2 px-3 py-1.5 focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary ${hasData ? 'cursor-pointer hover:bg-surface-container-low' : ''}`}
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
        <span className={`text-[10px] font-semibold uppercase ${levelColor}`}>
          {event.level}
        </span>
        <span className={`inline-flex w-fit items-center rounded px-1.5 py-0.5 text-[10px] font-semibold ${streamColors.bg} ${streamColors.text}`}>
          {event.stream}
        </span>
        <span className="flex min-w-0 items-start gap-1 text-xs text-on-surface">
          <span className="min-w-0 flex-1 break-all">{event.summary}</span>
          {hasData && (
            <span className={`shrink-0 text-on-surface-variant transition-transform ${expanded ? 'rotate-90' : ''}`}>
              &#9656;
            </span>
          )}
        </span>
      </div>
      {expanded && hasData && (
        <pre className="mx-3 mb-2 overflow-x-auto rounded-lg bg-surface-container-low px-3 py-2 text-[11px] text-on-surface-variant">
          {JSON.stringify(event.data, null, 2)}
        </pre>
      )}
    </div>
  );
}
