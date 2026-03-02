import { useState, useEffect, useRef, useCallback } from 'react';
import { useEventStore } from '../../lib/hooks/useEventStore';
import { StreamChips } from './StreamChips';
import { LevelFilter } from './LevelFilter';
import { EventRow } from './EventRow';
import { MetricsView } from './MetricsView';
import type { StreamName, LevelName } from '../../lib/events';

type Tab = 'events' | 'metrics';

export function LogViewerApp() {
  const { events, clear } = useEventStore();
  const [tab, setTab] = useState<Tab>('events');
  const [activeStreams, setActiveStreams] = useState<Set<StreamName>>(
    () => new Set(['pipeline', 'audio', 'system'])
  );
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
    e => activeStreams.has(e.stream as StreamName) && activeLevels.has(e.level as LevelName)
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
      .map(e => `${e.timestamp} [${e.stream}] ${e.level.toUpperCase()} ${e.summary}`)
      .join('\n');
    navigator.clipboard.writeText(text);
  }, [filteredEvents]);

  return (
    <div className="h-screen flex flex-col bg-white dark:bg-stone-900 text-stone-900 dark:text-stone-100">
      {/* Header */}
      <div className="shrink-0 border-b border-stone-200 dark:border-stone-700 px-4 py-3">
        <div className="flex items-center justify-between mb-3">
          {/* Tabs */}
          <div className="flex gap-1">
            {(['events', 'metrics'] as Tab[]).map((t) => (
              <button
                key={t}
                onClick={() => setTab(t)}
                className={`px-3 py-1.5 rounded-md text-xs font-medium transition-colors ${
                  tab === t
                    ? 'bg-stone-800 dark:bg-stone-200 text-white dark:text-stone-900'
                    : 'text-stone-500 dark:text-stone-400 hover:bg-stone-100 dark:hover:bg-stone-800'
                }`}
              >
                {t.charAt(0).toUpperCase() + t.slice(1)}
              </button>
            ))}
          </div>
          {/* Actions */}
          <div className="flex gap-2">
            <button
              onClick={handleCopyAll}
              className="px-3 py-1 rounded-md text-xs font-medium border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-800 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-700 transition-colors"
            >
              Copy All
            </button>
            <button
              onClick={clear}
              className="px-3 py-1 rounded-md text-xs font-medium border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-800 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-700 transition-colors"
            >
              Clear
            </button>
          </div>
        </div>
        {/* Filters (only for events tab) */}
        {tab === 'events' && (
          <div className="flex items-center justify-between">
            <StreamChips active={activeStreams} onToggle={toggleStream} />
            <LevelFilter active={activeLevels} onToggle={toggleLevel} />
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
            <div className="flex items-center justify-center h-32 text-stone-400 dark:text-stone-500 text-sm">
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
          <MetricsView events={events} />
        </div>
      )}
    </div>
  );
}
