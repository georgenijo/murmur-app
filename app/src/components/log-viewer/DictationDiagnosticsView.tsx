import { useCallback, useEffect, useState } from 'react';
import {
  armNextDictationCapture,
  deleteDictationCapture,
  disarmNextDictationCapture,
  getDictationCapture,
  getDictationCaptureStatus,
  listDictationCaptures,
  uploadDictationCapture,
  type BoundedPrivateTextV1,
  type DictationCaptureArmStatusV1,
  type DictationCaptureSummaryV1,
  type DictationCaptureV1,
} from '../../lib/dictationDiagnostics';

function displayTime(value: number): string {
  return new Date(value).toLocaleString();
}

function privateText(label: string, value: BoundedPrivateTextV1) {
  return (
    <section>
      <h3 className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-on-surface-variant">
        {label}{value.truncated ? ' · first 8 KB' : ''}
      </h3>
      <pre className="max-h-56 overflow-auto whitespace-pre-wrap rounded-lg bg-surface-container p-3 text-xs text-on-surface">
        {value.text || 'Empty'}
      </pre>
    </section>
  );
}

interface DictationDiagnosticsViewProps {
  active?: boolean;
  canArm?: boolean;
}

export function DictationDiagnosticsView({
  active = true,
  canArm = true,
}: DictationDiagnosticsViewProps) {
  const [status, setStatus] = useState<DictationCaptureArmStatusV1>({ state: 'unarmed' });
  const [captures, setCaptures] = useState<DictationCaptureSummaryV1[]>([]);
  const [selected, setSelected] = useState<DictationCaptureV1 | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());

  const refresh = useCallback(async () => {
    try {
      const [nextStatus, nextCaptures] = await Promise.all([
        getDictationCaptureStatus(),
        listDictationCaptures(),
      ]);
      setStatus(nextStatus);
      setCaptures(nextCaptures);
      setError(null);
    } catch {
      setError('Private dictation captures could not be refreshed.');
    }
  }, []);

  useEffect(() => {
    if (!active) return undefined;
    void refresh();
    const timer = window.setInterval(() => {
      setNow(Date.now());
      void refresh();
    }, 1_000);
    return () => window.clearInterval(timer);
  }, [active, refresh]);

  const arm = async () => {
    if (!window.confirm(
      'Capture exact text from the next live dictation?\n\n'
      + 'This applies once, expires in 10 minutes, and keeps the result privately on this Mac for at most 7 days. '
      + 'Nothing is uploaded until you review it and approve a separate upload.',
    )) return;
    try {
      setStatus(await armNextDictationCapture());
      setNotice('The next live dictation is armed for one private capture.');
      setError(null);
    } catch {
      setError('The one-shot private capture could not be armed.');
    }
  };

  const disarm = async () => {
    try {
      setStatus(await disarmNextDictationCapture());
      setNotice('The pending private capture was disarmed.');
    } catch {
      setError('The pending private capture could not be disarmed.');
    }
  };

  const review = async (captureId: string) => {
    try {
      setSelected(await getDictationCapture(captureId));
      setNotice(null);
      setError(null);
    } catch {
      setError('The private capture could not be opened.');
    }
  };

  const upload = async (capture: DictationCaptureV1) => {
    if (!window.confirm(
      'Upload this reviewed private capture?\n\n'
      + 'Its exact transcript text will be sent to the restricted Murmur diagnostics receiver and retained there for at most 7 days.',
    )) return;
    try {
      await uploadDictationCapture(capture.captureId);
      setNotice('Private capture uploaded. The local copy remains until you delete it.');
      setError(null);
    } catch {
      setError('The private capture was not uploaded. The local copy is unchanged.');
    }
  };

  const remove = async (captureId: string) => {
    if (!window.confirm('Delete this private local capture now? This cannot be undone.')) return;
    try {
      await deleteDictationCapture(captureId);
      if (selected?.captureId === captureId) setSelected(null);
      await refresh();
      setNotice('The local private capture was deleted.');
    } catch {
      setError('The private capture could not be deleted.');
    }
  };

  if (selected) {
    const result = selected.result;
    return (
      <div className="space-y-4 p-4">
        <button type="button" onClick={() => setSelected(null)} className="text-xs font-medium text-primary hover:underline">
          ← Back to private captures
        </button>
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h2 className="text-sm font-semibold text-on-surface">Private dictation capture · recording {selected.recordingId}</h2>
            <p className="text-[11px] text-error">This view contains exact transcript text and is never included in normal logs.</p>
          </div>
          <div className="flex gap-2">
            <button type="button" onClick={() => void upload(selected)} className="rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary">
              Upload reviewed capture
            </button>
            <button type="button" onClick={() => void remove(selected.captureId)} className="rounded-lg border border-error/20 px-3 py-1.5 text-xs font-medium text-error">
              Delete local copy
            </button>
          </div>
        </div>
        {notice && <p role="status" className="rounded-lg bg-primary/10 px-3 py-2 text-xs">{notice}</p>}
        {error && <p role="alert" className="rounded-lg bg-error/10 px-3 py-2 text-xs text-error">{error}</p>}
        {result.kind === 'success' ? (
          <>
            {privateText('Raw recognition', result.rawText)}
            {privateText('Final delivery', result.finalText)}
            <p className="text-[11px] text-on-surface-variant">Model {result.modelId} · {result.totalMs} ms</p>
          </>
        ) : (
          <p className="rounded-lg bg-surface-container p-3 text-xs">
            No transcript was captured. Outcome: {result.outcome}; reason: {result.errorCode}.
          </p>
        )}
      </div>
    );
  }

  const secondsRemaining = status.state === 'armed'
    ? Math.max(0, Math.ceil((status.expiresAtMs - now) / 1_000))
    : null;

  return (
    <div className="space-y-4 p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold text-on-surface">Private dictation capture</h2>
          <p className="text-[11px] text-on-surface-variant">Normal diagnostics never contain transcript text. Arm exactly one live recording only when text is needed to diagnose it.</p>
        </div>
        <div className="flex items-center gap-2">
          {status.state === 'armed' && <span className="text-xs">Armed · expires in {secondsRemaining}s</span>}
          {status.state === 'capturing' && <span className="text-xs">Capturing recording {status.recordingId}</span>}
          {canArm && status.state === 'armed' && (
            <button type="button" onClick={() => void disarm()} className="rounded-lg border border-outline-variant/20 px-3 py-1.5 text-xs">Disarm</button>
          )}
          {canArm && status.state === 'unarmed' && (
            <button type="button" onClick={() => void arm()} className="rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-on-primary">Capture next dictation</button>
          )}
        </div>
      </div>
      {!canArm && <p className="text-xs text-on-surface-variant">Arm a capture from Diagnostics in Murmur’s main window.</p>}
      {notice && <p role="status" className="rounded-lg bg-primary/10 px-3 py-2 text-xs">{notice}</p>}
      {error && <p role="alert" className="rounded-lg bg-error/10 px-3 py-2 text-xs text-error">{error}</p>}
      <div className="overflow-hidden rounded-lg border border-outline-variant/15">
        {captures.length === 0 ? (
          <p className="p-4 text-xs text-on-surface-variant">No private dictation captures on this Mac.</p>
        ) : captures.map(capture => (
          <div key={capture.captureId} className="flex items-center justify-between gap-3 border-t border-outline-variant/10 px-3 py-2 text-xs first:border-t-0">
            <span>Recording {capture.recordingId} · {capture.outcome} · {displayTime(capture.capturedAtMs)}</span>
            <button type="button" onClick={() => void review(capture.captureId)} className="font-medium text-primary hover:underline">Review</button>
          </div>
        ))}
      </div>
    </div>
  );
}
