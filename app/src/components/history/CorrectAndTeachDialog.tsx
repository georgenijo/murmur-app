import {
  useEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';
import {
  confirmLearnedCorrection,
  discardLearnedCorrectionProposal,
  proposeLearnedCorrection,
  proposeSpecificLearnedCorrection,
  type CorrectionProposalOutcome,
} from '../../lib/correctAndTeach';
import type { HistoryEntry } from '../../lib/history';
import { scopeLabel, type KnowledgeScope } from '../../lib/knowledge';

interface Props {
  entry: HistoryEntry;
  onClose: () => void;
  onSaveCorrection: (text: string) => void;
}

type DialogStep = 'edit' | 'automatic_review' | 'specific_edit' | 'specific_review';

export function CorrectAndTeachDialog({ entry, onClose, onSaveCorrection }: Props) {
  const [correctedText, setCorrectedText] = useState(entry.text);
  const [outcome, setOutcome] = useState<CorrectionProposalOutcome | null>(null);
  const [step, setStep] = useState<DialogStep>('edit');
  const [scope, setScope] = useState<KnowledgeScope>({ kind: 'global' });
  const [specificSource, setSpecificSource] = useState('');
  const [specificReplacement, setSpecificReplacement] = useState('');
  const [selection, setSelection] = useState({ start: 0, end: 0 });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const correctedTextareaRef = useRef<HTMLTextAreaElement>(null);
  const heardTextareaRef = useRef<HTMLTextAreaElement>(null);
  const specificSourceRef = useRef<HTMLInputElement>(null);
  const reviewHeadingRef = useRef<HTMLHeadingElement>(null);
  const proposalId = outcome?.kind === 'proposal' ? outcome.proposalId : null;
  const proposalIdRef = useRef<number | null>(proposalId);
  const closedRef = useRef(false);
  const onCloseRef = useRef(onClose);
  proposalIdRef.current = proposalId;
  onCloseRef.current = onClose;

  const discardCurrent = () => {
    if (proposalIdRef.current !== null) {
      void discardLearnedCorrectionProposal(proposalIdRef.current).catch(() => {});
      proposalIdRef.current = null;
    }
  };

  const close = () => {
    closedRef.current = true;
    discardCurrent();
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
        'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled])',
      ) ?? []);
      if (!nodes.length) return;
      const active = document.activeElement;
      if (!dialogRef.current?.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? nodes[nodes.length - 1] : nodes[0]).focus();
      } else if (event.shiftKey && active === nodes[0]) {
        event.preventDefault();
        nodes[nodes.length - 1].focus();
      } else if (!event.shiftKey && active === nodes[nodes.length - 1]) {
        event.preventDefault();
        nodes[0].focus();
      }
    };
    document.addEventListener('keydown', onKey);
    return () => {
      closedRef.current = true;
      document.removeEventListener('keydown', onKey);
      previous?.focus();
      discardCurrent();
    };
    // The dialog owns one proposal lifecycle; recreating this handler would
    // discard a proposal while the user is reviewing it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      if (step === 'edit') correctedTextareaRef.current?.focus();
      else if (step === 'specific_edit') specificSourceRef.current?.focus();
      else reviewHeadingRef.current?.focus();
    }, 40);
    return () => window.clearTimeout(timer);
  }, [step]);

  const reviewAutomatic = async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await proposeLearnedCorrection(entry.text, correctedText, entry.teachingContext);
      if (closedRef.current) {
        if (next.kind === 'proposal') await discardLearnedCorrectionProposal(next.proposalId).catch(() => {});
        return;
      }
      setOutcome(next);
      setStep('automatic_review');
      if (next.kind === 'proposal') setScope(next.scopeOptions[0].scope);
    } catch (cause) {
      if (!closedRef.current) setError(String(cause));
    } finally {
      if (!closedRef.current) setBusy(false);
    }
  };

  const openSpecificTerm = () => {
    if (outcome?.kind === 'proposal') {
      setSpecificSource(outcome.source);
      setSpecificReplacement(outcome.replacement);
    } else {
      setSpecificSource('');
      setSpecificReplacement('');
    }
    discardCurrent();
    setOutcome(null);
    setError(null);
    setStep('specific_edit');
  };

  const reviewSpecific = async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await proposeSpecificLearnedCorrection(
        entry.text,
        specificSource,
        specificReplacement,
        entry.teachingContext,
      );
      if (closedRef.current) {
        if (next.kind === 'proposal') await discardLearnedCorrectionProposal(next.proposalId).catch(() => {});
        return;
      }
      if (next.kind === 'unsafe') {
        setError(next.reason);
        return;
      }
      setOutcome(next);
      setScope(next.scopeOptions[0].scope);
      setStep('specific_review');
    } catch (cause) {
      if (!closedRef.current) setError(String(cause));
    } finally {
      if (!closedRef.current) setBusy(false);
    }
  };

  const useSelection = () => {
    if (selection.end <= selection.start) return;
    setSpecificSource(entry.text.slice(selection.start, selection.end));
    specificSourceRef.current?.focus();
  };

  const captureHeardSelection = () => {
    const textarea = heardTextareaRef.current;
    if (!textarea) return;
    setSelection({
      start: textarea.selectionStart,
      end: textarea.selectionEnd,
    });
  };

  const backToEdit = () => {
    discardCurrent();
    setOutcome(null);
    setError(null);
    setStep('edit');
  };

  const backToSpecificEdit = () => {
    discardCurrent();
    setOutcome(null);
    setError(null);
    setStep('specific_edit');
  };

  const saveOnly = () => {
    closedRef.current = true;
    discardCurrent();
    onSaveCorrection(correctedText);
    onClose();
  };

  const remember = async () => {
    if (outcome?.kind !== 'proposal') return;
    setBusy(true);
    setError(null);
    try {
      await confirmLearnedCorrection(outcome.proposalId, scope);
      proposalIdRef.current = null;
      closedRef.current = true;
      onSaveCorrection(correctedText);
      onClose();
    } catch (cause) {
      setError(String(cause));
    } finally {
      if (!closedRef.current) setBusy(false);
    }
  };

  const selectedScope = outcome?.kind === 'proposal'
    ? outcome.scopeOptions.find((option) => JSON.stringify(option.scope) === JSON.stringify(scope))
    : null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-3 backdrop-blur-[2px] sm:p-6" onMouseDown={(event) => { if (event.target === event.currentTarget) close(); }}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="correct-and-teach-title" className="max-h-[calc(100vh-1.5rem)] w-full max-w-[640px] overflow-y-auto rounded-2xl border border-outline-variant/30 bg-surface p-4 shadow-2xl sm:max-h-[88vh] sm:p-5">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 id="correct-and-teach-title" className="text-base font-semibold text-on-surface">Correct and Teach</h2>
            <p className="mt-1 text-xs text-on-surface-variant">Edit this local history entry. Murmur will not remember a rule unless you review it and press Remember correction.</p>
          </div>
          <button type="button" onClick={close} aria-label="Close Correct and Teach" className="rounded-md px-2 py-1 text-on-surface-variant hover:bg-surface-container">✕</button>
        </div>

        {step === 'edit' && (
          <div className="mt-4 space-y-4">
            <label className="block text-xs font-medium text-on-surface">Corrected transcript
              <textarea ref={correctedTextareaRef} aria-label="Corrected transcript" value={correctedText} onChange={(event) => setCorrectedText(event.target.value)} maxLength={8_192} className="mt-1 min-h-36 w-full resize-y rounded-xl border border-outline-variant/40 bg-surface-container-lowest px-3 py-2 text-sm leading-relaxed text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary" />
            </label>
            <p className="text-xs text-on-surface-variant">This changes history only. It cannot alter text that was already copied, pasted, or saved to a file.</p>
            {error && <Alert>{error}</Alert>}
            <div className="flex flex-wrap justify-end gap-2 border-t border-outline-variant/25 pt-4">
              <button type="button" onClick={close} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Cancel</button>
              <button type="button" onClick={() => void reviewAutomatic()} disabled={busy || !correctedText.trim() || correctedText === entry.text} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary disabled:opacity-40">{busy ? 'Reviewing…' : 'Review correction'}</button>
            </div>
          </div>
        )}

        {step === 'automatic_review' && outcome?.kind === 'unsafe' && (
          <div className="mt-4 space-y-4">
            <h3 ref={reviewHeadingRef} tabIndex={-1} className="sr-only">Automatic correction review</h3>
            <div className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm text-amber-950 dark:border-amber-800 dark:bg-amber-950/30 dark:text-amber-200">
              <strong>No automatic rule suggested.</strong>
              <p className="mt-1 text-xs">{outcome.reason}</p>
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <Example label="Before" text={entry.text} />
              <Example label="Your correction" text={correctedText} />
            </div>
            <SpecificTermAction onClick={openSpecificTerm} />
            <div className="flex flex-wrap justify-end gap-2 border-t border-outline-variant/25 pt-4">
              <button type="button" onClick={backToEdit} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Back</button>
              <button type="button" onClick={saveOnly} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary">Save correction only</button>
            </div>
          </div>
        )}

        {step === 'automatic_review' && outcome?.kind === 'proposal' && (
          <ReviewStep
            headingRef={reviewHeadingRef}
            outcome={outcome}
            scope={scope}
            selectedScope={selectedScope}
            busy={busy}
            error={error}
            onScope={setScope}
            onBack={backToEdit}
            onSaveOnly={saveOnly}
            onRemember={() => void remember()}
            specificAction={<SpecificTermAction onClick={openSpecificTerm} />}
          />
        )}

        {step === 'specific_edit' && (
          <div className="mt-4 space-y-4">
            <div className="rounded-xl border border-primary/25 bg-primary/5 p-3">
              <h3 className="text-sm font-semibold text-on-surface">Teach specific term</h3>
              <p className="mt-1 text-xs text-on-surface-variant">Choose the exact heard term or short phrase, then enter exactly how Murmur should write it.</p>
            </div>
            <label className="block text-xs font-medium text-on-surface">Select from the heard transcript
              <textarea
                ref={heardTextareaRef}
                readOnly
                aria-label="Heard transcript for term selection"
                aria-describedby="heard-selection-help"
                value={entry.text}
                onSelect={captureHeardSelection}
                onKeyUp={captureHeardSelection}
                onMouseUp={captureHeardSelection}
                className="mt-1 min-h-24 w-full resize-y rounded-xl border border-outline-variant/40 bg-surface-container-lowest px-3 py-2 text-sm leading-relaxed text-on-surface outline-none focus:border-primary focus:ring-1 focus:ring-primary"
              />
            </label>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p id="heard-selection-help" className="text-xs text-on-surface-variant">Mouse or keyboard selection works. Nothing is remembered when text is selected.</p>
              <button type="button" onClick={useSelection} disabled={selection.end <= selection.start} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium disabled:opacity-40">Use selected text</button>
            </div>
            <p role="status" aria-live="polite" className="sr-only">{selection.end > selection.start ? `${selection.end - selection.start} characters selected` : 'No heard text selected'}</p>
            <div className="grid gap-3 sm:grid-cols-2">
              <label className="block text-xs font-medium text-on-surface">Exact heard term
                <input ref={specificSourceRef} aria-label="Exact heard term" value={specificSource} onChange={(event) => setSpecificSource(event.target.value)} maxLength={256} className="mt-1 w-full rounded-lg border border-outline-variant/40 bg-surface-container-lowest px-3 py-2 text-sm outline-none focus:border-primary focus:ring-1 focus:ring-primary" />
              </label>
              <label className="block text-xs font-medium text-on-surface">Exact written replacement
                <input aria-label="Exact written replacement" value={specificReplacement} onChange={(event) => setSpecificReplacement(event.target.value)} maxLength={256} className="mt-1 w-full rounded-lg border border-outline-variant/40 bg-surface-container-lowest px-3 py-2 text-sm outline-none focus:border-primary focus:ring-1 focus:ring-primary" />
              </label>
            </div>
            <p className="text-xs text-on-surface-variant">Each side is limited to eight tokens and 256 characters. Heard text must match a whole term in this example.</p>
            {error && <Alert>{error}</Alert>}
            <div className="flex flex-wrap justify-end gap-2 border-t border-outline-variant/25 pt-4">
              <button type="button" onClick={backToEdit} disabled={busy} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Back</button>
              <button type="button" onClick={saveOnly} disabled={busy} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Save correction only</button>
              <button type="button" onClick={() => void reviewSpecific()} disabled={busy || !specificSource.trim() || !specificReplacement.trim()} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary disabled:opacity-40">{busy ? 'Reviewing…' : 'Review specific term'}</button>
            </div>
          </div>
        )}

        {step === 'specific_review' && outcome?.kind === 'proposal' && (
          <ReviewStep
            headingRef={reviewHeadingRef}
            outcome={outcome}
            scope={scope}
            selectedScope={selectedScope}
            busy={busy}
            error={error}
            onScope={setScope}
            onBack={backToSpecificEdit}
            onSaveOnly={saveOnly}
            onRemember={() => void remember()}
          />
        )}
      </div>
    </div>
  );
}

function SpecificTermAction({ onClick }: { onClick: () => void }) {
  return (
    <div className="rounded-xl border border-outline-variant/30 bg-surface-container-lowest p-3">
      <p className="text-xs text-on-surface-variant">Need a narrower rule? Choose the exact term or short phrase yourself.</p>
      <button type="button" onClick={onClick} className="mt-2 rounded-lg border border-primary/40 px-3 py-2 text-xs font-semibold text-primary hover:bg-primary/10">Teach specific term</button>
    </div>
  );
}

function ReviewStep({
  headingRef,
  outcome,
  scope,
  selectedScope,
  busy,
  error,
  onScope,
  onBack,
  onSaveOnly,
  onRemember,
  specificAction,
}: {
  headingRef: RefObject<HTMLHeadingElement>;
  outcome: Extract<CorrectionProposalOutcome, { kind: 'proposal' }>;
  scope: KnowledgeScope;
  selectedScope: { scope: KnowledgeScope; label: string } | null | undefined;
  busy: boolean;
  error: string | null;
  onScope: (scope: KnowledgeScope) => void;
  onBack: () => void;
  onSaveOnly: () => void;
  onRemember: () => void;
  specificAction?: ReactNode;
}) {
  return (
    <div className="mt-4 space-y-4">
      <h3 ref={headingRef} tabIndex={-1} className="sr-only">Review learned correction</h3>
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
        <select aria-label="Learned correction scope" value={JSON.stringify(scope)} onChange={(event) => onScope(JSON.parse(event.target.value) as KnowledgeScope)} className="mt-1 w-full rounded-lg border border-outline-variant/40 bg-surface-container-lowest px-3 py-2 text-sm outline-none focus:border-primary">
          {outcome.scopeOptions.map((option) => <option key={JSON.stringify(option.scope)} value={JSON.stringify(option.scope)}>{scopeLabel(option.scope)}{option.scope.kind === 'app' && option.label !== option.scope.bundleId ? ` (${option.label})` : ''}</option>)}
        </select>
      </label>
      <p className="text-xs text-on-surface-variant">Scope to be saved: <strong>{selectedScope ? scopeLabel(selectedScope.scope) : scopeLabel(scope)}</strong>. You can edit, disable, export, or delete the learned rule later in Settings → Knowledge.</p>
      {specificAction}
      {error && <Alert>{error}</Alert>}
      <div className="flex flex-wrap justify-end gap-2 border-t border-outline-variant/25 pt-4">
        <button type="button" onClick={onBack} disabled={busy} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Back</button>
        <button type="button" onClick={onSaveOnly} disabled={busy} className="rounded-lg border border-outline-variant/40 px-3 py-2 text-xs font-medium">Save correction only</button>
        <button type="button" onClick={onRemember} disabled={busy} className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-on-primary disabled:opacity-40">{busy ? 'Saving…' : 'Remember correction'}</button>
      </div>
    </div>
  );
}

function Alert({ children }: { children: ReactNode }) {
  return <p role="alert" className="rounded-lg bg-red-500/10 px-3 py-2 text-xs text-red-700 dark:text-red-300">{children}</p>;
}

function Example({ label, text }: { label: string; text: string }) {
  return <div className="rounded-xl border border-outline-variant/30 bg-surface-container-lowest p-3"><p className="text-[11px] font-semibold uppercase tracking-wide text-on-surface-variant">{label}</p><p className="mt-1 max-h-32 overflow-y-auto whitespace-pre-wrap text-xs leading-relaxed text-on-surface">{text}</p></div>;
}
