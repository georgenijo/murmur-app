interface ReviewAppliedProps {
  onUndo: () => void;
  /** Set when the last undo attempt failed — the text stays applied and Undo stays available. */
  errorMessage?: string | null;
}

/** Transient "applied" row: check + "Replaced" + Undo. Auto-dismisses via the driver's own timer. */
export function ReviewApplied({ onUndo, errorMessage }: ReviewAppliedProps) {
  return (
    <div className="flex items-center gap-2 px-3 pb-3 text-[12px] text-white/80">
      {errorMessage ? (
        <>
          <span className="text-amber-400" aria-hidden="true">!</span>
          <span className="text-amber-200/90">{errorMessage}</span>
        </>
      ) : (
        <>
          <span className="text-emerald-400" aria-hidden="true">✓</span>
          <span>Replaced</span>
        </>
      )}
      <button
        type="button"
        onClick={onUndo}
        className="ml-auto px-2 py-1 rounded-md bg-white/10 hover:bg-white/15 text-white/80 transition-colors"
      >
        Undo
      </button>
    </div>
  );
}
