import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { flog } from '../../lib/log';
import { useTransformReviewDriver } from '../../lib/hooks/useTransformReviewDriver';
import { isMockReviewEnabled, useMockReviewDriver } from '../../lib/hooks/useTransformReviewMockDriver';
import {
  REVIEW_APPROVE_PULSE_MS,
  REVIEW_DISMISS_MS,
  REVIEW_DISMISS_TRANSITION,
  REVIEW_ENTRANCE_FROM_SCALE,
  REVIEW_ENTRANCE_TRANSITION,
  REVIEW_DISMISS_TO_SCALE,
} from '../../lib/overlayMotion';
import { deriveReviewState } from './deriveReviewState';
import { ReviewChip } from './ReviewChip';
import { ReviewDiff } from './ReviewDiff';
import { ReviewActions } from './ReviewActions';
import { ReviewApplied } from './ReviewApplied';

/**
 * Composition shell for the transform review popover — the separate
 * `transform-review` Tauri window's React root (no shared context with the
 * main window or the overlay, same as `OverlayWidget`).
 *
 * Picks between the real event-driven driver and the dev-only mock driver
 * (both hooks are always called, gated internally by `enabled`, per the
 * Rules of Hooks — mirrors `useHoldDownToggle`/`useDoubleTapToggle`), derives
 * a pure view model via `deriveReviewState`, and wires the ready/failed-only
 * keyboard shortcuts (Enter=approve, Esc=cancel, Cmd+R=retry).
 */
export function TransformReviewApp() {
  const mockEnabled = isMockReviewEnabled();
  const mockDriver = useMockReviewDriver(mockEnabled);
  const realDriver = useTransformReviewDriver(!mockEnabled);
  const driver = mockEnabled ? mockDriver : realDriver;

  const vm = useMemo(
    () => deriveReviewState({
      state: driver.state,
      errorCode: driver.errorCode,
      instruction: driver.content.instruction,
      original: driver.content.original,
      proposed: driver.content.proposed,
      thinkingElapsedMs: driver.thinkingElapsedMs,
    }),
    [driver.state, driver.errorCode, driver.content, driver.thinkingElapsedMs],
  );

  // --- Motion: entrance on mount, soft dismiss on Cancel, a short pulse on Approve. ---
  const [mounted, setMounted] = useState(false);
  const [dismissing, setDismissing] = useState(false);
  const [pulsing, setPulsing] = useState(false);
  const reducedMotionRef = useRef(false);

  useEffect(() => {
    reducedMotionRef.current = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    const raf = requestAnimationFrame(() => setMounted(true));
    return () => cancelAnimationFrame(raf);
  }, []);

  const dismissThen = useCallback((action: () => void) => {
    if (reducedMotionRef.current) {
      action();
      return;
    }
    setDismissing(true);
    window.setTimeout(() => {
      action();
      setDismissing(false);
      setMounted(false);
      requestAnimationFrame(() => setMounted(true));
    }, REVIEW_DISMISS_MS);
  }, []);

  const handleCancel = useCallback(() => dismissThen(driver.cancel), [dismissThen, driver]);

  const handleApprove = useCallback(() => {
    if (!vm.approveEnabled) return;
    if (!reducedMotionRef.current) {
      setPulsing(true);
      window.setTimeout(() => setPulsing(false), REVIEW_APPROVE_PULSE_MS);
    }
    driver.approve();
  }, [driver, vm.approveEnabled]);

  const handleRetry = useCallback(() => {
    if (!vm.retryEnabled) return;
    driver.retry();
  }, [driver, vm.retryEnabled]);

  // --- Keyboard: Enter=approve, Esc=cancel, Cmd+R=retry — ready/failed only. ---
  useEffect(() => {
    if (!vm.keyboardActionsActive) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Enter' && vm.approveEnabled) {
        e.preventDefault();
        handleApprove();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        handleCancel();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'r' && vm.retryEnabled) {
        e.preventDefault();
        handleRetry();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [vm.keyboardActionsActive, vm.approveEnabled, vm.retryEnabled, handleApprove, handleCancel, handleRetry]);

  useEffect(() => {
    flog.info('transform-review', 'mounted', { mock: mockEnabled });
    return () => { flog.info('transform-review', 'unmounted'); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const scale = dismissing
    ? REVIEW_DISMISS_TO_SCALE
    : pulsing
      ? 1.01
      : mounted
        ? 1
        : REVIEW_ENTRANCE_FROM_SCALE;

  return (
    <div
      className="transform-review-surface w-full h-full overflow-hidden select-none"
      style={{
        background: 'rgba(20, 20, 20, 0.92)',
        borderRadius: 14,
        backdropFilter: 'blur(40px)',
        WebkitBackdropFilter: 'blur(40px)',
        boxShadow: '0 8px 32px rgba(0,0,0,0.35)',
        opacity: dismissing ? 0 : mounted ? 1 : 0,
        transform: `scale(${scale})`,
        transition: dismissing
          ? REVIEW_DISMISS_TRANSITION
          : `${REVIEW_ENTRANCE_TRANSITION}, transform ${REVIEW_APPROVE_PULSE_MS}ms ease-out`,
      }}
    >
      <ReviewChip vm={vm} />

      {vm.showDiff && <ReviewDiff original={driver.content.original} proposed={driver.content.proposed} />}

      {vm.errorMessage && (
        <div className="px-3 py-2 text-[12px] text-red-300/90">{vm.errorMessage}</div>
      )}

      {vm.showUndo && <ReviewApplied onUndo={driver.undo} />}

      {(vm.approveEnabled || vm.retryEnabled || vm.cancelEnabled) && (
        <ReviewActions
          approveEnabled={vm.approveEnabled}
          retryEnabled={vm.retryEnabled}
          cancelEnabled={vm.cancelEnabled}
          onApprove={handleApprove}
          onRetry={handleRetry}
          onCancel={handleCancel}
        />
      )}
    </div>
  );
}
