import { useState, useEffect, useRef, useCallback } from 'react';
import { useEventStore } from '../../lib/hooks/useEventStore';
import { useResourceMonitor } from '../../lib/hooks/useResourceMonitor';
import { StreamChips } from './StreamChips';
import { LevelFilter } from './LevelFilter';
import { EventRow } from './EventRow';
import { MetricsView } from './MetricsView';
import type { StreamName, LevelName } from '../../lib/events';
import { formatEventForCopy, matchesTransformPassId } from '../../lib/eventFilters';

type Tab = 'events' | 'metrics';

export function LogViewerApp() {
  const { events, clear } = useEventStore();
  const [tab, setTab] = useState<Tab>('events');
  const resourceReadings = useResourceMonitor(tab === 'metrics');
  const [activeStreams, setActiveStreams] = useState<Set<StreamName>>(
    () => new Set(['pipeline', 'audio', 'transform', 'system'])
  );
  const [transformPassId, setTransformPassId] = useState('');
  const [activeLevels, setActiveLevels] = useState<Set<LevelName>>(
    () => new Set(['info', 'warn', 'error'])
  );
  const listRef = useRef<HTMLDivElement>(null);
  const autoScrollRef = useRef(true);

  const toggleStream = useCallback((stream: StreamName) => {
    setActiveStreams(prev => {
      const next = new Set(prev);
      if (next.has(stream)) next.delete(stream);
      else next.add(stream);
      return next;
    });
  }, []);

  const toggleLevel = useCallback((level: LevelName) => {
    setActiveLevels(prev => {
      const next = new Set(prev);
      if (next.has(level)) next.delete(level);
      else next.add(level);
      return next;
    });
  }, []);

  const filteredEvents = events.filter(
    e => activeStreams.has(e.stream as StreamName)
      && activeLevels.has(e.level as LevelName)
      && matchesTransformPassId(e, transformPassId)
  );

  // Auto-scroll to bottom when new events arrive
  useEffect(() => {
    if (autoScrollRef.current && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [filteredEvents.length]);

  const handleScroll = useCallback(() => {
    if (!listRef.current) return;
    const { scrollTop, scrollHeight, clientHeight } = listRef.current;
    autoScrollRef.current = scrollHeight - scrollTop - clientHeight < 40;
  }, []);

  const handleCopyAll = useCallback(() => {
    const text = filteredEvents
      .map(formatEventForCopy)
      .join('\n');
    navigator.clipboard.writeText(text);
  }, [filteredEvents]);

  return (
    <div className="h-screen flex flex-col bg-background text-on-surface">
      {/* Header */}
      <div className="shrink-0 bg-surface-container-low px-4 py-3">
        <div className="flex items-center justify-between mb-3">
          {/* Tabs */}
          <div className="flex gap-1 rounded-xl bg-surface-container p-1">
            {(['events', 'metrics'] as Tab[]).map((t) => (
              <button
                key={t}
                onClick={() => setTab(t)}
                className={`rounded-lg px-3 py-1.5 text-xs font-medium transition-[background-color,box-shadow,color] ${
                  tab === t
                    ? 'bg-surface-container-lowest text-on-surface shadow-sm'
                    : 'text-on-surface-variant hover:text-on-surface'
                }`}
                aria-pressed={tab === t}
              >
                {t.charAt(0).toUpperCase() + t.slice(1)}
              </button>
            ))}
          </div>
          {/* Actions */}
          <div className="flex gap-2">
            <button
              onClick={handleCopyAll}
              className="rounded-lg border border-outline-variant/10 bg-surface-container-lowest px-3 py-1.5 text-xs font-medium text-on-surface-variant shadow-sm transition-colors hover:bg-surface-container hover:text-primary focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              Copy All
            </button>
            <button
              onClick={clear}
              className="rounded-lg border border-outline-variant/10 bg-surface-container-lowest px-3 py-1.5 text-xs font-medium text-on-surface-variant shadow-sm transition-colors hover:bg-surface-container hover:text-error focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              Clear
            </button>
          </div>
        </div>
        {/* Filters (only for events tab) */}
        {tab === 'events' && (
          <div className="flex flex-wrap items-center justify-between gap-2">
            <StreamChips active={activeStreams} onToggle={toggleStream} />
            <div className="flex items-center gap-2">
              <label className="flex items-center gap-1.5 text-xs text-on-surface-variant">
                Pass ID
                <input
                  type="text"
                  inputMode="numeric"
                  pattern="[0-9]*"
                  value={transformPassId}
                  onChange={(event) => setTransformPassId(event.target.value)}
                  placeholder="All"
                  aria-label="Filter by transform pass ID"
                  className="w-20 rounded-lg border border-outline-variant/20 bg-surface-container-lowest px-2 py-1 font-mono text-xs text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
                />
              </label>
              <LevelFilter active={activeLevels} onToggle={toggleLevel} />
            </div>
          </div>
        )}
      </div>

      {/* Content */}
      {tab === 'events' ? (
        <div
          ref={listRef}
          onScroll={handleScroll}
          className="flex-1 overflow-y-auto font-mono text-xs"
        >
          {filteredEvents.length === 0 ? (
            <div className="flex items-center justify-center h-32 text-on-surface-variant text-sm">
              No events to display
            </div>
          ) : (
            filteredEvents.map((event, i) => (
              <EventRow key={`${event.timestamp}-${i}`} event={event} />
            ))
          )}
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto">
          <MetricsView events={events} resourceReadings={resourceReadings} />
        </div>
      )}
    </div>
  );
}
