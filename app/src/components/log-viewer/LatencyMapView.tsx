import { useMemo, useState } from 'react';
import {
  clearUiLatencySamples,
  getUiLatencyBuild,
  summarizeUiLatency,
  useUiLatencySamples,
  type UiLatencySampleV1,
} from '../../lib/uiLatency';

interface LatencyMapViewProps {
  samples?: UiLatencySampleV1[];
}

function percentile(values: number[], fraction: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)];
}

function formatDuration(milliseconds: number): string {
  if (milliseconds < 10) return `${milliseconds.toFixed(1)} ms`;
  return `${Math.round(milliseconds)} ms`;
}

function shortView(view: string): string {
  return view
    .replace('settings.model.diagnostics.', 'diagnostics.')
    .replace('settings.text.editor.', 'text editor.');
}

export function LatencyMapView({ samples: suppliedSamples }: LatencyMapViewProps) {
  const liveSamples = useUiLatencySamples();
  const samples = suppliedSamples ?? liveSamples;
  const currentBuild = getUiLatencyBuild();
  const builds = useMemo(
    () => Array.from(new Set(samples.map(sample => sample.build))).sort().reverse(),
    [samples],
  );
  const [selectedBuild, setSelectedBuild] = useState(currentBuild);
  const [copyStatus, setCopyStatus] = useState<'idle' | 'copied' | 'failed'>('idle');
  const [confirmClear, setConfirmClear] = useState(false);
  const filtered = useMemo(
    () => samples.filter(sample => sample.build === selectedBuild),
    [samples, selectedBuild],
  );
  const edges = useMemo(() => summarizeUiLatency(filtered), [filtered]);
  const paintedValues = filtered.map(sample => sample.paintedMs);
  const commitValues = filtered.map(sample => sample.commitMs);
  const firstFrameValues = filtered.flatMap(sample =>
    sample.firstFrameMs === undefined ? [] : [sample.firstFrameMs]);
  const frameIntervals = filtered.flatMap(sample =>
    sample.frameIntervalMs === undefined ? [] : [sample.frameIntervalMs]);
  const median = percentile(paintedValues, 0.5);
  const p95 = percentile(paintedValues, 0.95);
  const medianCommit = percentile(commitValues, 0.5);
  const medianFirstFrame = percentile(firstFrameValues, 0.5);
  const p95FirstFrame = percentile(firstFrameValues, 0.95);
  const medianFrameInterval = percentile(frameIntervals, 0.5);

  const copyReport = async () => {
    const report = {
      schemaVersion: 1,
      generatedAt: new Date().toISOString(),
      build: selectedBuild,
      summary: {
        sampleCount: filtered.length,
        edgeCount: edges.length,
        medianPaintedMs: median,
        p95PaintedMs: p95,
        medianCommitMs: medianCommit,
        medianFirstFrameMs: firstFrameValues.length > 0 ? medianFirstFrame : null,
        p95FirstFrameMs: firstFrameValues.length > 0 ? p95FirstFrame : null,
        medianFrameIntervalMs: frameIntervals.length > 0 ? medianFrameInterval : null,
      },
      edges,
      samples: filtered,
    };
    try {
      await navigator.clipboard.writeText(JSON.stringify(report, null, 2));
      setCopyStatus('copied');
    } catch {
      setCopyStatus('failed');
    }
  };

  return (
    <div className="flex flex-col gap-5 p-4">
      <section aria-labelledby="ui-latency-heading">
        <div className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 id="ui-latency-heading" className="text-sm font-semibold text-on-surface">
              UI latency map
            </h2>
            <p className="mt-0.5 max-w-2xl text-[11px] leading-relaxed text-on-surface-variant">
              First frame is the primary JS-visible response metric. The second-frame paint proxy remains for regression continuity; content-free samples stay on this Mac.
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <label className="flex items-center gap-2 text-[11px] text-on-surface-variant">
              Build
              <select
                value={selectedBuild}
                onChange={event => setSelectedBuild(event.target.value)}
                aria-label="UI latency build"
                className="rounded-lg border border-on-surface-variant bg-surface-container-lowest px-2.5 py-1.5 text-xs text-on-surface outline-none focus:border-primary"
              >
                {!builds.includes(currentBuild) && <option value={currentBuild}>{currentBuild}</option>}
                {builds.map(build => <option key={build} value={build}>{build}</option>)}
              </select>
            </label>
            <button
              type="button"
              onClick={() => void copyReport()}
              disabled={filtered.length === 0}
              className="rounded-lg border border-outline-variant/20 bg-surface-container-lowest px-2.5 py-1.5 text-xs font-medium text-on-surface-variant hover:text-primary disabled:cursor-not-allowed disabled:opacity-50"
            >
              {copyStatus === 'copied' ? 'Copied JSON' : copyStatus === 'failed' ? 'Copy failed' : 'Copy JSON'}
            </button>
            {confirmClear ? (
              <>
                <button
                  type="button"
                  onClick={() => {
                    clearUiLatencySamples();
                    setConfirmClear(false);
                  }}
                  className="rounded-lg bg-error px-2.5 py-1.5 text-xs font-semibold text-on-error"
                >
                  Confirm clear
                </button>
                <button
                  type="button"
                  onClick={() => setConfirmClear(false)}
                  className="rounded-lg px-2.5 py-1.5 text-xs font-medium text-on-surface-variant hover:bg-surface-container"
                >
                  Cancel
                </button>
              </>
            ) : (
              <button
                type="button"
                onClick={() => setConfirmClear(true)}
                disabled={samples.length === 0}
                className="rounded-lg border border-outline-variant/20 bg-surface-container-lowest px-2.5 py-1.5 text-xs font-medium text-on-surface-variant hover:text-error disabled:cursor-not-allowed disabled:opacity-50"
              >
                Clear
              </button>
            )}
          </div>
        </div>

        <div className="mt-3 grid grid-cols-2 gap-3 md:grid-cols-4 xl:grid-cols-7">
          {[
            ['Transitions', String(filtered.length)],
            ['Route edges', String(edges.length)],
            ['Median commit', filtered.length > 0 ? formatDuration(medianCommit) : '—'],
            ['Median first frame', firstFrameValues.length > 0 ? formatDuration(medianFirstFrame) : '—'],
            ['P95 first frame', firstFrameValues.length > 0 ? formatDuration(p95FirstFrame) : '—'],
            ['P95 paint', filtered.length > 0 ? formatDuration(p95) : '—'],
            ['Median frame', frameIntervals.length > 0 ? formatDuration(medianFrameInterval) : '—'],
          ].map(([label, value]) => (
            <div key={label} className="rounded-xl border border-outline-variant/10 bg-surface-container-lowest p-3 shadow-sm">
              <div className="text-[10px] font-bold uppercase tracking-[0.12em] text-on-surface-variant">{label}</div>
              <div className="mt-1 text-lg font-semibold tabular-nums text-on-surface">{value}</div>
            </div>
          ))}
        </div>
      </section>

      <section aria-labelledby="route-edges-heading">
        <div className="mb-2">
          <h3 id="route-edges-heading" className="text-sm font-semibold text-on-surface">Measured route edges</h3>
          <p className="text-[11px] text-on-surface-variant">Sorted by slowest P95 first frame. Commit isolates React work; the paint proxy includes an additional frame boundary.</p>
        </div>
        {edges.length === 0 ? (
          <div className="rounded-xl border border-dashed border-outline-variant/30 bg-surface-container-low p-8 text-center">
            <div className="text-sm font-medium text-on-surface">No transitions for this build yet</div>
            <p className="mt-1 text-xs text-on-surface-variant">Move between History, Settings pages, editors, and Diagnostics tabs to populate the map.</p>
          </div>
        ) : (
          <div className="overflow-x-auto rounded-xl border border-outline-variant/20 bg-surface-container-lowest">
            <table className="w-full min-w-[860px] border-collapse text-left text-xs">
              <thead className="bg-surface-container-low text-[10px] font-bold uppercase tracking-[0.1em] text-on-surface-variant">
                <tr>
                  <th className="px-3 py-2.5">From → To</th>
                  <th className="px-3 py-2.5 text-right">Count</th>
                  <th className="px-3 py-2.5 text-right">Median commit</th>
                  <th className="px-3 py-2.5 text-right">Median first</th>
                  <th className="px-3 py-2.5 text-right">P95 first</th>
                  <th className="px-3 py-2.5 text-right">Frames</th>
                  <th className="px-3 py-2.5 text-right">Median paint</th>
                  <th className="px-3 py-2.5 text-right">P95 paint</th>
                </tr>
              </thead>
              <tbody>
                {edges.map(edge => (
                  <tr key={`${edge.from}-${edge.to}`} className="border-t border-outline-variant/15">
                    <td className="px-3 py-3">
                      <div className="flex items-center gap-2 whitespace-nowrap font-mono text-[11px]">
                        <span className="rounded-md bg-surface-container px-2 py-1 text-on-surface-variant">{shortView(edge.from)}</span>
                        <span aria-hidden="true" className="text-primary">→</span>
                        <span className="rounded-md bg-primary/10 px-2 py-1 text-on-surface">{shortView(edge.to)}</span>
                      </div>
                    </td>
                    <td className="px-3 py-3 text-right tabular-nums text-on-surface-variant">{edge.count}</td>
                    <td className="px-3 py-3 text-right tabular-nums text-on-surface">{formatDuration(edge.medianCommitMs)}</td>
                    <td className="px-3 py-3 text-right font-semibold tabular-nums text-on-surface">{edge.medianFirstFrameMs === null ? '—' : formatDuration(edge.medianFirstFrameMs)}</td>
                    <td className="px-3 py-3 text-right font-semibold tabular-nums text-on-surface">{edge.p95FirstFrameMs === null ? '—' : formatDuration(edge.p95FirstFrameMs)}</td>
                    <td className="px-3 py-3 text-right tabular-nums text-on-surface-variant">{edge.medianFrameCount === null ? '—' : edge.medianFrameCount}</td>
                    <td className="px-3 py-3 text-right tabular-nums text-on-surface">{formatDuration(edge.medianPaintedMs)}</td>
                    <td className="px-3 py-3 text-right tabular-nums text-on-surface-variant">{formatDuration(edge.p95PaintedMs)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {filtered.length > 0 && (
        <section aria-labelledby="recent-ui-latency-heading">
          <h3 id="recent-ui-latency-heading" className="mb-2 text-sm font-semibold text-on-surface">Recent transitions</h3>
          <div className="space-y-1.5">
            {[...filtered].slice(-8).reverse().map(sample => (
              <div key={sample.sampleId} className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg bg-surface-container-low px-3 py-2 text-[11px]">
                <span className="min-w-0 flex-1 truncate font-mono text-on-surface">
                  {shortView(sample.from)} → {shortView(sample.to)}
                </span>
                <span className="text-on-surface-variant">{sample.trigger}</span>
                <span className="tabular-nums text-on-surface-variant">commit {formatDuration(sample.commitMs)}</span>
                {sample.firstFrameMs !== undefined && (
                  <span className="font-semibold tabular-nums text-on-surface">first {formatDuration(sample.firstFrameMs)}</span>
                )}
                {sample.frameIntervalMs !== undefined && (
                  <span className="tabular-nums text-on-surface-variant">frame {formatDuration(sample.frameIntervalMs)}</span>
                )}
                <span className="tabular-nums text-on-surface-variant">paint {formatDuration(sample.paintedMs)}</span>
              </div>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
