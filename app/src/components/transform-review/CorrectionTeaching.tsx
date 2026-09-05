import { useEffect, useRef, useState } from 'react';
import {
  proposeLearnedCorrection,
  confirmLearnedCorrection,
  discardLearnedCorrectionProposal,
  type CorrectionProposalOutcome,
  type TeachingContext,
} from '../../lib/correctAndTeach';

type TeachingState =
  | { kind: 'idle' }
  | { kind: 'loading' }
  | { kind: 'review'; proposal: Extract<CorrectionProposalOutcome, { kind: 'proposal' }> }
  | { kind: 'saving' }
  | { kind: 'saved' }
  | { kind: 'error'; message: string };

export function CorrectionTeaching({ original, proposed, context }: {
  original: string; proposed: string; context?: TeachingContext;
}) {
  const [state, setState] = useState<TeachingState>({ kind: 'idle' });
  const [scopeIndex, setScopeIndex] = useState(0);
  const alive = useRef(true);
  const proposalId = useRef<number | null>(null);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      if (proposalId.current !== null) {
        void discardLearnedCorrectionProposal(proposalId.current).catch(() => {});
      }
    };
  }, []);

  const review = async () => {
    setState({ kind: 'loading' });
    try {
      const result = await proposeLearnedCorrection(original, proposed, context);
      if (!alive.current) {
        if (result.kind === 'proposal') void discardLearnedCorrectionProposal(result.proposalId).catch(() => {});
        return;
      }
      if (result.kind === 'unsafe') setState({ kind: 'error', message: result.reason });
      else {
        proposalId.current = result.proposalId;
        setScopeIndex(0);
        setState({ kind: 'review', proposal: result });
      }
    } catch {
      if (alive.current) setState({ kind: 'error', message: 'Could not prepare a reusable correction.' });
    }
  };

  const remember = async () => {
    if (state.kind !== 'review') return;
    const option = state.proposal.scopeOptions[scopeIndex];
    if (!option) return;
    const id = state.proposal.proposalId;
    setState({ kind: 'saving' });
    try {
      await confirmLearnedCorrection(id, option.scope);
      proposalId.current = null;
      if (alive.current) setState({ kind: 'saved' });
    } catch {
      if (alive.current) setState({ kind: 'error', message: 'Could not save this correction. Review conflicts in Text & Vocabulary.' });
    }
  };

  return (
    <div className="mx-3 mb-2 border-t border-white/10 pt-2 text-[11px] text-white/70"
      onKeyDown={(event) => event.stopPropagation()}>
      {state.kind === 'idle' && <button type="button" onClick={() => void review()}
        className="rounded px-1 py-1 underline focus-visible:outline focus-visible:outline-white">Remember this correction…</button>}
      {state.kind === 'loading' && <span role="status">Preparing correction…</span>}
      {state.kind === 'saving' && <span role="status">Saving correction…</span>}
      {state.kind === 'saved' && <span role="status">Remembered for future dictations.</span>}
      {state.kind === 'error' && <span role="alert">{state.message}</span>}
      {state.kind === 'review' && <>
        <p className="mb-1 break-words">Remember “{state.proposal.source}” → “{state.proposal.replacement}”</p>
        <label>Apply to{' '}
          <select value={scopeIndex} onChange={(event) => setScopeIndex(Number(event.target.value))}
            className="rounded bg-neutral-800 px-1 py-1 text-white">
            {state.proposal.scopeOptions.map((option, index) => <option key={index} value={index}>{option.label}</option>)}
          </select>
        </label>
        <button type="button" onClick={() => void remember()}
          className="ml-2 rounded bg-white/15 px-2 py-1 text-white">Remember correction</button>
      </>}
    </div>
  );
}
