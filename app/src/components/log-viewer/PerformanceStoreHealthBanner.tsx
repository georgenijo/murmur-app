import type {
  PerformanceStoreErrorClassV1,
  PerformanceStoreHealthV1,
  PerformanceStoreOperationV1,
} from '../../lib/performance';

interface PerformanceStoreHealthBannerProps {
  health: PerformanceStoreHealthV1 | null;
  loading: boolean;
  error: string | null;
  recovering: boolean;
  recoveryError: string | null;
  onRefresh: () => void;
  onRecover: () => void;
}

const ERROR_CLASS_LABELS: Record<PerformanceStoreErrorClassV1, string> = {
  busyLocked: 'the local store stayed busy',
  storageFull: 'local storage space was unavailable',
  readOnly: 'the local store was read-only',
  io: 'a local storage operation failed',
  corruptIntegrity: 'the local store failed an integrity check',
  schemaMigration: 'the local store schema could not be opened safely',
  invalidRecord: 'an invalid diagnostics record was rejected',
  unavailable: 'the local store was unavailable',
};

const OPERATION_LABELS: Record<PerformanceStoreOperationV1, string> = {
  initialize: 'startup',
  begin: 'run start',
  update: 'run update',
  complete: 'run completion',
  read: 'read',
  write: 'write',
  clear: 'clear',
};

function unavailableGuidance(health: PerformanceStoreHealthV1): string {
  switch (health.recommendedAction) {
    case 'freeDisk':
      return 'Free local disk space, then retry. Dictation remains available, but new diagnostics are not being saved.';
    case 'checkPermissions':
      return 'Check that Murmur can write to its Application Support data, then retry. Dictation remains available.';
    case 'reinitializeStore':
      return 'Reinitialize the diagnostics store to quarantine the unreadable copy and start a new one. Transcription history, settings, and logs are not changed.';
    case 'restartApp':
      return 'Quit and reopen Murmur to retry local diagnostics startup. Dictation remains available.';
    case 'retry':
      return 'Retry the local diagnostics store. Dictation remains available, but new diagnostics may be skipped.';
    case 'none':
      return 'Quit and reopen Murmur. Dictation remains available, but new diagnostics are not being saved.';
  }
}

function shouldOfferRecovery(health: PerformanceStoreHealthV1): boolean {
  return health.recommendedAction === 'retry'
    || health.recommendedAction === 'freeDisk'
    || health.recommendedAction === 'checkPermissions'
    || health.recommendedAction === 'reinitializeStore';
}

function recoveryButtonLabel(health: PerformanceStoreHealthV1, recovering: boolean): string {
  if (recovering) return 'Recovering…';
  return health.recommendedAction === 'reinitializeStore'
    ? 'Reinitialize Store…'
    : 'Retry Store';
}

export function PerformanceStoreHealthBanner({
  health,
  loading,
  error,
  recovering,
  recoveryError,
  onRefresh,
  onRecover,
}: PerformanceStoreHealthBannerProps) {
  if (loading && !health) {
    return (
      <div role="status" className="rounded-xl border border-outline-variant/15 bg-surface-container-low px-3 py-2">
        <div className="text-xs font-medium text-on-surface">Checking diagnostics store…</div>
      </div>
    );
  }

  if (error || !health) {
    return (
      <div role="alert" className="rounded-xl border border-warning/20 bg-warning/10 px-3 py-2">
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="text-xs font-semibold text-warning">Diagnostics health could not be verified</div>
            <p className="mt-0.5 text-[11px] text-on-surface-variant">
              Refresh to check the local diagnostics store. Dictation remains available.
            </p>
          </div>
          <button type="button" onClick={onRefresh} className="shrink-0 text-xs font-semibold text-warning underline">
            Refresh
          </button>
        </div>
      </div>
    );
  }

  if (health.status === 'unavailable') {
    const recover = () => {
      if (health.recommendedAction === 'reinitializeStore') {
        const confirmed = window.confirm(
          'Reinitialize the local diagnostics store?\n\n'
          + 'Murmur will quarantine the unreadable diagnostics database and start a new one. '
          + 'This removes Performance runs and resource samples only; transcription history, settings, logs, and benchmark reports are not changed.',
        );
        if (!confirmed) return;
      }
      onRecover();
    };
    return (
      <div role="alert" className="rounded-xl border border-error/20 bg-error/10 px-3 py-2">
        <div className="flex items-start justify-between gap-3">
          <div>
            <div className="text-xs font-semibold text-error">Diagnostics store unavailable</div>
            <p className="mt-0.5 text-[11px] text-on-surface-variant">{unavailableGuidance(health)}</p>
            {recoveryError && (
              <p className="mt-1 text-[11px] font-medium text-error">
                Diagnostics recovery did not complete. Dictation remains available.
              </p>
            )}
          </div>
          {shouldOfferRecovery(health) && (
            <button
              type="button"
              disabled={recovering}
              onClick={recover}
              className="shrink-0 text-xs font-semibold text-error underline disabled:opacity-50"
            >
              {recoveryButtonLabel(health, recovering)}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (health.skippedRunCount > 0) {
    const failure = health.lastFailure;
    const latestWriteWasNotSaved = failure?.retryExhausted === true
      && failure.operation !== 'begin';
    return (
      <div role="status" className="rounded-xl border border-warning/20 bg-warning/10 px-3 py-2">
        <div className="text-xs font-semibold text-warning">
          {health.skippedRunCount} diagnostics {health.skippedRunCount === 1 ? 'run was' : 'runs were'} skipped
        </div>
        <p className="mt-0.5 text-[11px] text-on-surface-variant">
          Dictation continued normally, and the diagnostics store is available now.
          {failure?.operation === 'begin' && ` The latest run start was skipped because ${ERROR_CLASS_LABELS[failure.errorClass]} after ${failure.attemptCount} ${failure.attemptCount === 1 ? 'attempt' : 'attempts'}.`}
          {health.lastRecovery && ' Murmur also quarantined the unreadable diagnostics store and started a new one.'}
        </p>
        {latestWriteWasNotSaved && (
          <p className="mt-1 text-[11px] text-on-surface-variant">
            <span className="font-semibold text-warning">Recent diagnostics data was not saved.</span>
            {' '}The latest {OPERATION_LABELS[failure.operation]} did not finish because {ERROR_CLASS_LABELS[failure.errorClass]} after {failure.attemptCount} {failure.attemptCount === 1 ? 'attempt' : 'attempts'}.
          </p>
        )}
      </div>
    );
  }

  if (health.lastFailure?.retryExhausted) {
    const failure = health.lastFailure;
    return (
      <div role="status" className="rounded-xl border border-warning/20 bg-warning/10 px-3 py-2">
        <div className="text-xs font-semibold text-warning">Recent diagnostics data was not saved</div>
        <p className="mt-0.5 text-[11px] text-on-surface-variant">
          Dictation continued normally, and the diagnostics store is available now. The latest {OPERATION_LABELS[failure.operation]} did not finish because {ERROR_CLASS_LABELS[failure.errorClass]} after {failure.attemptCount} {failure.attemptCount === 1 ? 'attempt' : 'attempts'}.
          {health.lastRecovery && ' Murmur also quarantined the unreadable diagnostics store and started a new one.'}
        </p>
      </div>
    );
  }

  if (health.lastRecovery) {
    return (
      <div role="status" className="rounded-xl border border-primary/20 bg-primary/10 px-3 py-2">
        <div className="text-xs font-semibold text-on-surface">Diagnostics store recovered</div>
        <p className="mt-0.5 text-[11px] text-on-surface">
          Murmur quarantined an unreadable diagnostics store and started a new one. Dictation data, settings, and logs were not changed.
        </p>
      </div>
    );
  }

  return (
    <div role="status" className="rounded-xl border border-success/20 bg-success/10 px-3 py-2">
      <div className="text-xs font-semibold text-success">Diagnostics store available</div>
      <p className="mt-0.5 text-[11px] text-on-surface-variant">
        New content-free performance runs and resource samples are being saved locally.
      </p>
    </div>
  );
}
