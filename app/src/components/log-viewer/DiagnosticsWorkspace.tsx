import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { useEventStore } from '../../lib/hooks/useEventStore';
import { usePerformanceDiagnostics } from '../../lib/hooks/usePerformanceDiagnostics';
import { usePerformanceHealth } from '../../lib/hooks/usePerformanceHealth';
import { StreamChips } from './StreamChips';
import { LevelFilter } from './LevelFilter';
import { EventRow } from './EventRow';
import { PerformanceView } from './PerformanceView';
import { RunsView } from './RunsView';
import { ReportCompareView } from './ReportCompareView';
import { TransformDiagnosticsView } from './TransformDiagnosticsView';
import { LatencyMapView } from './LatencyMapView';
import { LEVELS, STREAMS, type StreamName, type LevelName } from '../../lib/events';
import { beginCurrentUiTransition, useUiLatencyDestination } from '../../lib/uiLatency';
import {
  CORRELATION_FIELD_LABELS,
  formatEventForCopy,
  matchesCorrelation,
  type CorrelationField,
  type CorrelationFilter,
} from '../../lib/eventFilters';

export type DiagnosticsTab = 'events' | 'performance' | 'latency' | 'runs' | 'transforms' | 'reports';
const MAX_RENDERED_EVENTS = 100;
const TABS: { id: DiagnosticsTab; label: string }[] = [
  { id: 'events', label: 'Events' },
  { id: 'runs', label: 'Runs' },
  { id: 'performance', label: 'Performance' },
  { id: 'latency', label: 'Latency' },
  { id: 'reports', label: 'Compare' },
  { id: 'transforms', label: 'Transform' },
];

export function isDiagnosticsTab(value: string): value is DiagnosticsTab {
  return TABS.some(tab => tab.id === value);
}

interface DiagnosticsWorkspaceProps {
  active?: boolean;
  requestedTab?: DiagnosticsTab;
  onPopOut?: (tab: DiagnosticsTab) => void;
}

export function DiagnosticsWorkspace({
  active = true,
  requestedTab,
  onPopOut,
}: DiagnosticsWorkspaceProps) {
  const { events, clear } = useEventStore(active);
  const [tab, setTab] = useState<DiagnosticsTab>(requestedTab ?? 'events');
  useUiLatencyDestination(active ? `settings.model.diagnostics.${tab}` : null);
  const performance = usePerformanceDiagnostics(active);
  const health = usePerformanceHealth(active && tab === 'performance');
  const [activeStreams, setActiveStreams] = useState<Set<StreamName>>(
    () => new Set(['pipeline', 'audio', 'transform', 'system'])
  );
  const [correlation, setCorrelation] = useState<CorrelationFilter>({
    field: 'transform_pass_id',
    value: '',
  });
  const [activeLevels, setActiveLevels] = useState<Set<LevelName>>(
    () => new Set(['info', 'warn', 'error'])
  );
  const listRef = useRef<HTMLDivElement>(null);
  const autoScrollRef = useRef(true);

  useEffect(() => {
    if (requestedTab) setTab(requestedTab);
  }, [requestedTab]);

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

  const filteredEvents = useMemo(() => events.filter(
    e => activeStreams.has(e.stream as StreamName)
      && activeLevels.has(e.level as LevelName)
      && matchesCorrelation(e, correlation)
  ), [activeLevels, activeStreams, correlation, events]);
  const renderedEvents = useMemo(
    () => filteredEvents.slice(-MAX_RENDERED_EVENTS),
    [filteredEvents],
  );
  const renderedEventOffset = filteredEvents.length - renderedEvents.length;

  // Auto-scroll to bottom when new events arrive
  useEffect(() => {
    if (!active || tab !== 'events') return;
    if (autoScrollRef.current && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [active, renderedEvents.length, tab]);

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

  const showCorrelatedEvents = useCallback((filter: CorrelationFilter) => {
    setCorrelation(filter);
    setActiveStreams(new Set(STREAMS));
    setActiveLevels(new Set(LEVELS));
    setTab('events');
  }, []);

  return (
    <div className="flex h-full min-h-0 flex-col bg-background text-on-surface">
      {/* Header */}
      <div className="shrink-0 bg-surface-container-low px-4 py-3">
        <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
          {/* Tabs */}
          <div role="tablist" aria-label="Diagnostics views" className="flex max-w-full gap-1 overflow-x-auto rounded-xl bg-surface-container p-1">
            {TABS.map(({ id, label }) => (
              <button
                key={id}
                type="button"
                role="tab"
                id={`diagnostics-tab-${id}`}
                aria-controls={`diagnostics-panel-${id}`}
                aria-selected={tab === id}
                onClick={() => {
                  if (tab !== id) {
                    beginCurrentUiTransition(`settings.model.diagnostics.${id}`, 'pointer');
                    setTab(id);
                  }
                }}
                className={`shrink-0 rounded-lg px-2.5 py-1.5 text-xs font-medium transition-[background-color,box-shadow,color] ${
                  tab === id
                    ? 'bg-surface-container-lowest text-on-surface shadow-sm'
                    : 'text-on-surface-variant hover:text-on-surface'
                }`}
              >
                {label}
              </button>
            ))}
          </div>
          {/* Actions */}
          {(onPopOut || tab === 'events') && (
            <div className="flex flex-wrap gap-2">
              {onPopOut && (
                <button
                  type="button"
                  onClick={() => onPopOut(tab)}
                  aria-label="Open Diagnostics in a separate window"
                  className="rounded-lg border border-outline-variant/10 bg-surface-container-lowest px-3 py-1.5 text-xs font-medium text-on-surface-variant shadow-sm transition-colors hover:bg-surface-container hover:text-primary focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                >
                  <span aria-hidden="true">↗</span> Pop out
                </button>
              )}
              {tab === 'events' && (
                <>
              <button
                type="button"
                onClick={handleCopyAll}
                className="rounded-lg border border-outline-variant/10 bg-surface-container-lowest px-3 py-1.5 text-xs font-medium text-on-surface-variant shadow-sm transition-colors hover:bg-surface-container hover:text-primary focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
              >
                Copy filtered Events
              </button>
              <button
                type="button"
                onClick={clear}
                className="rounded-lg border border-outline-variant/10 bg-surface-container-lowest px-3 py-1.5 text-xs font-medium text-on-surface-variant shadow-sm transition-colors hover:bg-surface-container hover:text-error focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
              >
                Clear Events
              </button>
                </>
              )}
            </div>
          )}
        </div>
        {/* Filters (only for events tab) */}
        {tab === 'events' && (
          <div className="flex flex-wrap items-center justify-between gap-2">
            <StreamChips active={activeStreams} onToggle={toggleStream} />
            <div className="flex flex-wrap items-center gap-2">
              <label className="flex items-center gap-1.5 text-xs text-on-surface-variant">
                Correlation
                <select
                  value={correlation.field}
                  onChange={event => setCorrelation(current => ({
                    field: event.target.value as CorrelationField,
                    value: current.value,
                  }))}
                  aria-label="Correlation field"
                  className="rounded-lg border border-on-surface-variant bg-surface-container-lowest px-2 py-1 text-xs text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
                >
                  {(Object.keys(CORRELATION_FIELD_LABELS) as CorrelationField[]).map(field => (
                    <option key={field} value={field}>{CORRELATION_FIELD_LABELS[field]}</option>
                  ))}
                </select>
                <input
                  type="text"
                  inputMode={correlation.field === 'run_id' ? 'text' : 'numeric'}
                  pattern={correlation.field === 'run_id' ? undefined : '[0-9]*'}
                  value={correlation.value}
                  onChange={(event) => setCorrelation(current => ({
                    ...current,
                    value: event.target.value,
                  }))}
                  placeholder="All"
                  aria-label={`Filter by ${CORRELATION_FIELD_LABELS[correlation.field]}`}
                  className="w-28 rounded-lg border border-on-surface-variant bg-surface-container-lowest px-2 py-1 font-mono text-xs text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
                />
              </label>
              <LevelFilter active={activeLevels} onToggle={toggleLevel} />
            </div>
          </div>
        )}
      </div>

      {/* Content */}
      {tab === 'events' && (
        <div
          role="tabpanel"
          id="diagnostics-panel-events"
          aria-labelledby="diagnostics-tab-events"
          ref={listRef}
          onScroll={handleScroll}
          className="flex-1 overflow-y-auto font-mono text-xs"
        >
          {filteredEvents.length === 0 ? (
            <div className="flex items-center justify-center h-32 text-on-surface-variant text-sm">
              No events to display
            </div>
          ) : (
            <>
              {renderedEvents.length < filteredEvents.length && (
                <div className="border-b border-outline-variant/15 bg-surface-container-low px-3 py-2 font-sans text-[11px] text-on-surface-variant">
                  Showing the newest {renderedEvents.length} of {filteredEvents.length} events. Copy includes every filtered event.
                </div>
              )}
              {renderedEvents.map((event, index) => (
                <EventRow
                  key={`${event.timestamp}-${renderedEventOffset + index}`}
                  event={event}
                />
              ))}
            </>
          )}
        </div>
      )}
      {tab === 'performance' && (
        <div
          role="tabpanel"
          id="diagnostics-panel-performance"
          aria-labelledby="diagnostics-tab-performance"
          className="flex-1 overflow-y-auto"
        >
          <PerformanceView
            samples={performance.samples}
            loading={performance.resourcesLoading}
            error={performance.resourcesError}
            health={health}
            onRetry={() => {
              void performance.refreshResources();
              health.refresh();
            }}
          />
        </div>
      )}
      {tab === 'latency' && (
        <div
          role="tabpanel"
          id="diagnostics-panel-latency"
          aria-labelledby="diagnostics-tab-latency"
          className="flex-1 overflow-y-auto"
        >
          <LatencyMapView />
        </div>
      )}
      {tab === 'runs' && (
        <div
          role="tabpanel"
          id="diagnostics-panel-runs"
          aria-labelledby="diagnostics-tab-runs"
          className="flex-1 overflow-y-auto"
        >
          <RunsView
            runs={performance.runs}
            loading={performance.runsLoading}
            error={performance.runsError}
            cleared={performance.cleared}
            clearing={performance.clearing}
            clearError={performance.clearError}
            onRetry={performance.refreshRuns}
            onClear={performance.clear}
            onShowEvents={showCorrelatedEvents}
          />
        </div>
      )}
      {tab === 'transforms' && (
        <div
          role="tabpanel"
          id="diagnostics-panel-transforms"
          aria-labelledby="diagnostics-tab-transforms"
          className="flex-1 overflow-y-auto"
        >
          <TransformDiagnosticsView />
        </div>
      )}
      <div
        role="tabpanel"
        id="diagnostics-panel-reports"
        aria-labelledby="diagnostics-tab-reports"
        hidden={tab !== 'reports'}
        className="flex-1 overflow-y-auto"
      >
        <ReportCompareView />
      </div>
    </div>
  );
}
