import { useCallback, useEffect, useState } from 'react';
import {
  armNextTransformCapture,
  deleteTransformCapture,
  getCaptureArmStatus,
  getTransformCapture,
  listTransformAttempts,
  listTransformCaptures,
  type CaptureArmStatusV1,
  type DiagnosticCaptureSummaryV1,
  type DiagnosticCaptureV1,
  type TransformAttemptV1,
} from '../../lib/transformDiagnostics';

function displayTime(value: number): string {
  return new Date(value).toLocaleString();
}

function textPanel(label: string, value: string | null) {
  return (
    <section>
      <h4 className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-on-surface-variant">{label}</h4>
      <pre className="max-h-44 overflow-auto whitespace-pre-wrap rounded-lg bg-surface-container p-3 text-xs text-on-surface">
        {value ?? 'Not captured for this attempt'}
      </pre>
    </section>
  );
}

export function TransformDiagnosticsView() {
  const [attempts, setAttempts] = useState<TransformAttemptV1[]>([]);
  const [captures, setCaptures] = useState<DiagnosticCaptureSummaryV1[]>([]);
  const [arm, setArm] = useState<CaptureArmStatusV1>({ armed: false, expiresAtMs: null });
  const [selected, setSelected] = useState<DiagnosticCaptureV1 | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [nextAttempts, nextCaptures, status] = await Promise.all([
        listTransformAttempts(),
        listTransformCaptures(),
        getCaptureArmStatus(),
      ]);
      setAttempts(nextAttempts);
      setCaptures(nextCaptures);
      setArm(status);
    } catch {
      setError('Transform diagnostics could not be refreshed.');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const armCapture = async () => {
    const confirmed = window.confirm(
      'Capture the next transform with exact text?\n\n'
      + 'The selected text, recognized instruction, generated output, and phase trace will be stored only on this Mac. '
      + 'The arm expires in 10 minutes, applies to one attempt, keeps at most 3 captures for 7 days, and never enters ordinary logs or copied diagnostics.',
    );
    if (!confirmed) return;
    try {
      setArm(await armNextTransformCapture());
      setError(null);
    } catch {
      setError('The one-shot diagnostic capture could not be armed.');
    }
  };

  const reviewCapture = async (captureId: string) => {
    try {
      setSelected(await getTransformCapture(captureId));
      setError(null);
    } catch {
      setError('The local diagnostic capture could not be opened.');
    }
  };

  const deleteCapture = async (capture: DiagnosticCaptureSummaryV1) => {
    if (!window.confirm('Delete this local content capture now? This cannot be undone.')) return;
    try {
      await deleteTransformCapture(capture.captureId);
      if (selected?.captureId === capture.captureId) setSelected(null);
      await refresh();
    } catch {
      setError('The local diagnostic capture could not be deleted.');
    }
  };

  if (selected) {
    return (
      <div className="space-y-4 p-4">
        <div className="flex items-start justify-between gap-3">
          <div>
            <button type="button" onClick={() => setSelected(null)} className="text-xs font-medium text-primary hover:underline">
              ← Back to transform diagnostics
            </button>
            <h2 className="mt-2 text-sm font-semibold text-on-surface">Local content capture · pass {selected.transformPassId}</h2>
            <p className="text-[11px] text-error">Private content — review here only. This view has no export action.</p>
          </div>
          <button
            type="button"
            onClick={() => void deleteCapture(selected)}
            className="rounded-lg border border-error/20 px-3 py-1.5 text-xs font-medium text-error hover:bg-error/10"
          >
            Delete capture
          </button>
        </div>
        {textPanel('Selected text', selected.selection)}
        {textPanel('Recognized instruction', selected.instruction)}
        {textPanel('Generated output', selected.output)}
        <section>
          <h4 className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-on-surface-variant">Phase trace</h4>
          <div className="overflow-hidden rounded-lg border border-outline-variant/15">
            {selected.phases.map((phase, index) => (
              <div key={`${phase.phase}-${index}`} className="grid grid-cols-[1fr_auto_auto] gap-3 border-t border-outline-variant/10 px-3 py-2 text-xs first:border-t-0">
                <span>{phase.phase}</span>
                <span>{phase.outcome}</span>
                <span className="font-mono text-on-surface-variant">
                  {phase.durationMs === null ? '—' : `${phase.durationMs} ms`}
                </span>
              </div>
            ))}
          </div>
        </section>
      </div>
    );
  }

  return (
    <div className="space-y-5 p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold text-on-surface">Transform diagnostics</h2>
          <p className="text-[11px] text-on-surface-variant">
            Every attempt is recorded without text. Exact content capture is explicit, one-shot, local, and expiring.
          </p>
        </div>
        <div className="flex items-center gap-2">
          {arm.armed && (
            <span className="rounded-full bg-amber-100 px-2.5 py-1 text-[11px] font-medium text-amber-800 dark:bg-amber-900/35 dark:text-amber-200">
              Next transform armed{arm.expiresAtMs ? ` until ${new Date(arm.expiresAtMs).toLocaleTimeString()}` : ''}
            </span>
          )}
          <button type="button" onClick={() => void refresh()} className="rounded-lg border border-outline-variant/15 px-3 py-1.5 text-xs">
            Refresh
          </button>
          <button type="button" disabled={arm.armed} onClick={() => void armCapture()} className="rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary disabled:opacity-50">
            Capture next transform
          </button>
        </div>
      </div>

      {error && <div role="alert" className="rounded-lg border border-error/20 bg-error/10 px-3 py-2 text-xs text-error">{error}</div>}

      <section>
        <h3 className="mb-2 text-xs font-semibold text-on-surface">Consented local captures</h3>
        {captures.length === 0 ? (
          <div className="rounded-xl border border-dashed border-outline-variant/25 p-4 text-xs text-on-surface-variant">
            No exact-content captures stored.
          </div>
        ) : (
          <div className="overflow-hidden rounded-xl border border-outline-variant/15">
            {captures.map(capture => (
              <div key={capture.captureId} className="flex items-center justify-between gap-3 border-t border-outline-variant/10 px-3 py-2 first:border-t-0">
                <div className="text-xs">
                  <span className="font-semibold">Pass {capture.transformPassId}</span>
                  <span className="ml-2 text-on-surface-variant">{capture.outcome} · {displayTime(capture.capturedAtMs)}</span>
                </div>
                <div className="flex gap-2">
                  <button type="button" onClick={() => void reviewCapture(capture.captureId)} className="text-xs font-medium text-primary hover:underline">Review</button>
                  <button type="button" onClick={() => void deleteCapture(capture)} className="text-xs font-medium text-error hover:underline">Delete</button>
                </div>
              </div>
            ))}
          </div>
        )}
      </section>

      <section>
        <h3 className="mb-2 text-xs font-semibold text-on-surface">Content-free attempts</h3>
        {loading && attempts.length === 0 ? (
          <div className="h-24 animate-pulse rounded-xl bg-surface-container" />
        ) : attempts.length === 0 ? (
          <div className="rounded-xl border border-dashed border-outline-variant/25 p-4 text-xs text-on-surface-variant">
            No transform attempts recorded yet.
          </div>
        ) : (
          <div className="overflow-x-auto rounded-xl border border-outline-variant/15">
            <table className="w-full min-w-[760px] text-left text-xs">
              <thead className="bg-surface-container-low text-[10px] uppercase tracking-wider text-on-surface-variant">
                <tr><th className="px-3 py-2">Pass</th><th className="px-3 py-2">Outcome</th><th className="px-3 py-2">Selection</th><th className="px-3 py-2">Model</th><th className="px-3 py-2">Phases</th></tr>
              </thead>
              <tbody>
                {attempts.map(attempt => (
                  <tr key={`${attempt.transformPassId}-${attempt.startedAtMs}`} className="border-t border-outline-variant/10 align-top">
                    <td className="px-3 py-2"><span className="font-semibold">{attempt.transformPassId}</span><span className="block text-[10px] text-on-surface-variant">{displayTime(attempt.startedAtMs)}</span></td>
                    <td className="px-3 py-2">{attempt.outcome}</td>
                    <td className="px-3 py-2">{attempt.selectionResult ?? '—'} · {attempt.selectionSource ?? '—'} · {attempt.selectionSizeBucket ?? '—'}</td>
                    <td className="px-3 py-2">{attempt.modelWarmState ?? '—'} · {attempt.outputTokenCount ?? '—'} tokens</td>
                    <td className="px-3 py-2 font-mono text-[10px] text-on-surface-variant">{attempt.phases.map(phase => `${phase.phase}:${phase.outcome}`).join(' → ') || '—'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </div>
  );
}
