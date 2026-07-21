interface ReviewActionsProps {
  approveEnabled: boolean;
  retryEnabled: boolean;
  cancelEnabled: boolean;
  onApprove: () => void;
  onRetry: () => void;
  onCancel: () => void;
}

/**
 * Approve / Retry / Cancel action row (ready and failed states). Purely
 * presentational — a button only renders when its `*Enabled` flag is set, so
 * `failed` (no Approve) and `ready` (all three) render correctly from the
 * same component.
 */
export function ReviewActions({
  approveEnabled,
  retryEnabled,
  cancelEnabled,
  onApprove,
  onRetry,
  onCancel,
}: ReviewActionsProps) {
  return (
    <div className="flex items-center gap-2 px-3 pb-3 pt-1 text-[11px]">
      {approveEnabled && (
        <button
          type="button"
          onClick={onApprove}
          className="px-2 py-1 rounded-md bg-emerald-500/20 text-emerald-300 hover:bg-emerald-500/30 transition-colors"
        >
          Approve <span className="text-white/40 ml-1">⏎</span>
        </button>
      )}
      {retryEnabled && (
        <button
          type="button"
          onClick={onRetry}
          className="px-2 py-1 rounded-md bg-white/10 text-white/70 hover:bg-white/15 transition-colors"
        >
          Retry <span className="text-white/40 ml-1">⌘R</span>
        </button>
      )}
      {cancelEnabled && (
        <button
          type="button"
          onClick={onCancel}
          className="px-2 py-1 rounded-md text-white/50 hover:text-white/70 hover:bg-white/10 transition-colors ml-auto"
        >
          Cancel <span className="text-white/40 ml-1">esc</span>
        </button>
      )}
    </div>
  );
}
