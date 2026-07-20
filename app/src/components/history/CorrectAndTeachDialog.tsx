import { useEffect, useRef, useState } from 'react';
import {
  confirmLearnedCorrection,
  discardLearnedCorrectionProposal,
  proposeLearnedCorrection,
  type CorrectionProposalOutcome,
} from '../../lib/correctAndTeach';
import type { HistoryEntry } from '../../lib/history';
import { scopeLabel, type KnowledgeScope } from '../../lib/knowledge';

interface Props {
  entry: HistoryEntry;
  onClose: () => void;
  onSaveCorrection: (text: string) => void;
}

export function CorrectAndTeachDialog({ entry, onClose, onSaveCorrection }: Props) {
  const [correctedText, setCorrectedText] = useState(entry.text);
  const [outcome, setOutcome] = useState<CorrectionProposalOutcome | null>(null);
  const [scope, setScope] = useState<KnowledgeScope>({ kind: 'global' });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const proposalId = outcome?.kind === 'proposal' ? outcome.proposalId : null;
  const proposalIdRef = useRef<number | null>(proposalId);
  const closedRef = useRef(false);
  const onCloseRef = useRef(onClose);
  proposalIdRef.current = proposalId;
  onCloseRef.current = onClose;

  const close = () => {
    closedRef.current = true;
    if (proposalIdRef.current !== null) void discardLearnedCorrectionProposal(proposalIdRef.current).catch(() => {});
    onCloseRef.current();
  };

  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        close();
        return;
      }
      if (event.key !== 'Tab') return;
      const nodes = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), textarea:not([disabled]), select:not([disabled])',
      ) ?? []);
      if (!nodes.length) return;
      if (event.shiftKey && document.activeElement === nodes[0]) {
        event.preventDefault();
        nodes[nodes.length - 1].focus();
      } else if (!event.shiftKey && document.activeElement === nodes[nodes.length - 1]) {
        event.preventDefault();
        nodes[0].focus();
      }
    };
    document.addEventListener('keydown', onKey);
    const timer = window.setTimeout(() => textareaRef.current?.focus(), 40);
    return () => {
      closedRef.current = true;
      document.removeEventListener('keydown', onKey);
      window.clearTimeout(timer);
      previous?.focus();
      if (proposalIdRef.current !== null) void discardLearnedCorrectionProposal(proposalIdRef.current).catch(() => {});
    };
    // The dialog owns one proposal lifecycle; recreating this handler would
    // discard a proposal while the user is reviewing it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const review = async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await proposeLearnedCorrection(entry.text, correctedText, entry.teachingContext);
      if (closedRef.current) {
        if (next.kind === 'proposal') await discardLearnedCorrectionProposal(next.proposalId).catch(() => {});
        return;
      }
      setOutcome(next);
      if (next.kind === 'proposal') setScope(next.scopeOptions[0].scope);
    } catch (cause) {
      if (!closedRef.current) setError(String(cause));
    } finally {
      if (!closedRef.current) setBusy(false);
    }
  };

  const saveOnly = () => {
    if (proposalId !== null) void discardLearnedCorrectionProposal(proposalId).catch(() => {});
    onSaveCorrection(correctedText);
    onClose();
  };

  const remember = async () => {
    if (outcome?.kind !== 'proposal') return;
    setBusy(true);
    setError(null);
    try {
      await confirmLearnedCorrection(outcome.proposalId, scope);
      onSaveCorrection(correctedText);
      onClose();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  const selectedScope = outcome?.kind === 'proposal'
    ? outcome.scopeOptions.find((option) => JSON.stringify(option.scope) === JSON.stringify(scope))
    : null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-6 backdrop-blur-[2px]" onMouseDown={(event) => { if (event.target === event.currentTarget) close(); }}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="correct-and-teach-title" className="max-h-[88vh] w-full max-w-[640px] overflow-y-auto rounded-2xl border border-outline-variant/30 bg-surface p-5 shadow-2xl">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 id="correct-and-teach-title" className="text-base font-semibold text-on-surface">Correct and Teach</h2>
            <p className="mt-1 text-xs text-on-surface-variant">Edit this local history entry. Murmur will not remember a rule unless you review it and press Remember correction.</p>
          </div>
          <button type="button" onClick={close} aria-label="Close Correct and Teach" className="rounded-md px-2 py-1 text-on-surface-variant hover:bg-surface-container">✕</button>
        </div>

        {outcome === null ? (
          <div className="mt-4 space-y-4">
            <label className="block text-xs font-medium text-on-surface">Corrected transcript
              <textarea ref={textareaRef} aria-label="Corrected transcript" value={correctedText} onChange={(event) => setCorrectedText(event.target.value)} maxLength={8_192} className="mt-1 min-h-36 w-full resize-y rounded-xl border border-outline-variant/40 bg-surface-container-lowest px-3 py-2 text-sm leading-relaxed text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary" />
            </label>
            <p className="text-xs text-on-surface-variant">This changes history only. It cannot alter text that was already copied, pasted, or saved to a file.</p>
            {error && <p role="alert" className="rounded-lg bg-red-500/10 px-3 py-2 text-xs text-red-700 dark:text-red-300">{error}</p>}
            <div className="flex justify-end gap-2 border-t border-outline-variant/25 pt-4">
              <button type="button" onClick={close} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Cancel</button>
              <button type="button" onClick={() => void review()} disabled={busy || !correctedText.trim() || correctedText === entry.text} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary disabled:opacity-40">{busy ? 'Reviewing…' : 'Review correction'}</button>
            </div>
          </div>
        ) : outcome.kind === 'unsafe' ? (
          <div className="mt-4 space-y-4">
            <div className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm text-amber-950 dark:border-amber-800 dark:bg-amber-950/30 dark:text-amber-200">
              <strong>No automatic rule suggested.</strong>
              <p className="mt-1 text-xs">{outcome.reason}</p>
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <Example label="Before" text={entry.text} />
              <Example label="Your correction" text={correctedText} />
            </div>
            <div className="flex justify-end gap-2 border-t border-outline-variant/25 pt-4">
              <button type="button" onClick={() => setOutcome(null)} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Back</button>
              <button type="button" onClick={saveOnly} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary">Save correction only</button>
            </div>
          </div>
        ) : (
          <div className="mt-4 space-y-4">
            <div className="rounded-xl border border-primary/30 bg-primary/5 p-4">
              <p className="text-[11px] font-semibold uppercase tracking-wide text-primary">Exact rule</p>
              <p className="mt-2 break-words text-sm text-on-surface"><span className="rounded bg-surface-container px-1.5 py-1 font-mono">{outcome.source}</span> <span aria-hidden="true">→</span> <span className="rounded bg-surface-container px-1.5 py-1 font-mono">{outcome.replacement}</span></p>
              <p className="mt-2 text-xs text-on-surface-variant">Affects {outcome.occurrenceCount} exact {outcome.occurrenceCount === 1 ? 'occurrence' : 'occurrences'} in this example. Matching is local, case-insensitive, and word-boundary constrained.</p>
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <Example label="Before" text={outcome.originalText} />
              <Example label="After" text={outcome.correctedText} />
            </div>
            <label className="block text-xs font-medium text-on-surface">Use this correction in
              <select aria-label="Learned correction scope" value={JSON.stringify(scope)} onChange={(event) => setScope(JSON.parse(event.target.value) as KnowledgeScope)} className="mt-1 w-full rounded-lg border border-outline-variant/40 bg-surface-container-lowest px-3 py-2 text-sm outline-none focus:border-primary">
                {outcome.scopeOptions.map((option) => <option key={JSON.stringify(option.scope)} value={JSON.stringify(option.scope)}>{scopeLabel(option.scope)}{option.scope.kind === 'app' && option.label !== option.scope.bundleId ? ` (${option.label})` : ''}</option>)}
              </select>
            </label>
            <p className="text-xs text-on-surface-variant">Scope to be saved: <strong>{selectedScope ? scopeLabel(selectedScope.scope) : scopeLabel(scope)}</strong>. You can edit, disable, export, or delete the learned rule later in Settings → Knowledge.</p>
            {error && <p role="alert" className="rounded-lg bg-red-500/10 px-3 py-2 text-xs text-red-700 dark:text-red-300">{error}</p>}
            <div className="flex flex-wrap justify-end gap-2 border-t border-outline-variant/25 pt-4">
              <button type="button" onClick={() => { void discardLearnedCorrectionProposal(outcome.proposalId).catch(() => {}); setOutcome(null); }} disabled={busy} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Back</button>
              <button type="button" onClick={saveOnly} disabled={busy} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Save correction only</button>
              <button type="button" onClick={() => void remember()} disabled={busy} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary disabled:opacity-40">{busy ? 'Saving…' : 'Remember correction'}</button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function Example({ label, text }: { label: string; text: string }) {
  return <div className="rounded-xl border border-outline-variant/30 bg-surface-container-lowest p-3"><p className="text-[11px] font-semibold uppercase tracking-wide text-on-surface-variant">{label}</p><p className="mt-1 max-h-32 overflow-y-auto whitespace-pre-wrap text-xs leading-relaxed text-on-surface">{text}</p></div>;
}
