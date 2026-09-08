"use client";

import { Check, Trash2 } from "lucide-react";
import {
  animate,
  motion,
  useAnimationControls,
  useMotionValue,
  useReducedMotion,
  useTransform,
} from "motion/react";
import { useEffect, useRef, useState } from "react";

import { cn } from "@/lib/sona-utils";

/**
 * A press-and-release under this duration is treated as a discrete click
 * (arm/confirm) rather than an abandoned hold attempt.
 */
const CLICK_THRESHOLD_MS = 250;

export interface HoldToDeleteButtonProps {
  /** Text displayed inside the button. */
  label?: string;
  /**
   * Duration in milliseconds the user must hold before the action triggers.
   * @default 2000
   */
  holdDuration?: number;
  /**
   * Duration in milliseconds the success state is visible before auto-resetting.
   * @default 1200
   */
  successDuration?: number;
  /** Called once when the hold completes. */
  onDelete?: () => void;
  /** Whether the button ignores interaction. @default false */
  disabled?: boolean;
  /** Additional CSS classes for the button. */
  className?: string;
  /**
   * Label shown after a discrete click arms the confirm state, awaiting a
   * second discrete click. @default `${label}?`
   */
  confirmLabel?: string;
  /**
   * Duration in milliseconds the armed confirm state stays active before it
   * automatically disarms. @default 4000
   */
  confirmTimeout?: number;
}

export default function HoldToDeleteButton({
  label = "Hold To Delete",
  holdDuration = 2000,
  successDuration = 1200,
  onDelete,
  disabled = false,
  className,
  confirmLabel,
  confirmTimeout = 4000,
}: HoldToDeleteButtonProps) {
  const [isHolding, setIsHolding] = useState(false);
  const [isArmed, setIsArmed] = useState(false);
  const [isCompleted, setIsCompleted] = useState(false);
  const holdTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const successTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const confirmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const startedAtRef = useRef(0);
  // Captures whether the button was already armed at the moment the current
  // press started, so a fast second press commits instead of re-arming.
  const wasArmedRef = useRef(false);
  const buttonControls = useAnimationControls();
  const progress = useMotionValue(0);
  const shouldReduceMotion = useReducedMotion();
  const resolvedHoldDuration = Math.max(0, holdDuration);
  const resolvedSuccessDuration = Math.max(0, successDuration);
  const resolvedConfirmTimeout = Math.max(0, confirmTimeout);
  const resolvedConfirmLabel = confirmLabel ?? `${label}?`;
  const progressClipPath = useTransform(
    progress,
    (value) => `inset(0 ${100 - value * 100}% 0 0)`,
  );

  const clearHoldTimer = () => {
    if (holdTimerRef.current) clearTimeout(holdTimerRef.current);
    holdTimerRef.current = null;
  };

  const clearConfirmTimer = () => {
    if (confirmTimerRef.current) clearTimeout(confirmTimerRef.current);
    confirmTimerRef.current = null;
  };

  const disarm = () => {
    clearConfirmTimer();
    setIsArmed(false);
  };

  const arm = () => {
    clearConfirmTimer();
    setIsArmed(true);
    confirmTimerRef.current = setTimeout(() => {
      confirmTimerRef.current = null;
      setIsArmed(false);
    }, resolvedConfirmTimeout);
  };

  const commit = () => {
    clearHoldTimer();
    clearConfirmTimer();
    setIsHolding(false);
    setIsArmed(false);
    setIsCompleted(true);
    buttonControls.start({
      transform: "translateX(0) scale(1)",
      transition: { duration: 0.16, ease: [0.23, 1, 0.32, 1] },
    });
    progress.set(1);
    onDelete?.();
  };

  // Shared "abandoned hold" feedback: resets the fill and plays the shake
  // animation proportional to how far the hold got.
  const runCancelAnimation = (elapsedMs: number) => {
    animate(progress, 0, {
      type: "spring",
      duration: 0.3,
      bounce: 0,
    });

    const heldRatio =
      resolvedHoldDuration === 0
        ? 1
        : Math.min(elapsedMs / resolvedHoldDuration, 1);

    if (shouldReduceMotion || heldRatio < 0.15) {
      buttonControls.start({ transform: "translateX(0) scale(1)" });
      return;
    }

    const isPastHalfway = heldRatio >= 0.5;
    buttonControls.start(
      {
        transform: isPastHalfway
          ? [
              "translateX(0) rotate(0deg) scale(1)",
              "translateX(-7px) rotate(-1.2deg) scale(0.985)",
              "translateX(6px) rotate(1deg) scale(0.99)",
              "translateX(-4px) rotate(-0.6deg) scale(0.995)",
              "translateX(2px) rotate(0.3deg) scale(1)",
              "translateX(0) rotate(0deg) scale(1)",
            ]
          : [
              "translateX(0) scale(1)",
              "translateX(-3px) scale(0.99)",
              "translateX(3px) scale(0.995)",
              "translateX(0) scale(1)",
            ],
      },
      {
        duration: isPastHalfway ? 0.38 : 0.24,
        ease: [0.23, 1, 0.32, 1],
      },
    );
  };

  /**
   * Involuntary interruption of an in-progress hold: pointer capture lost,
   * pointer dragged off, focus lost, app backgrounded, or Escape pressed.
   * Never treated as a click — always cancels the hold and disarms any
   * pending confirm state.
   */
  const forceCancelHold = () => {
    if (!holdTimerRef.current) return;
    const elapsedMs = performance.now() - startedAtRef.current;
    clearHoldTimer();
    setIsHolding(false);
    disarm();
    runCancelAnimation(elapsedMs);
  };

  const handleBlur = () => {
    forceCancelHold();
    disarm();
  };

  const resetState = () => {
    clearHoldTimer();
    if (successTimerRef.current) clearTimeout(successTimerRef.current);
    successTimerRef.current = null;
    setIsCompleted(false);
    buttonControls.start({ transform: "translateX(0) scale(1)" });
    animate(progress, 0, {
      type: "spring",
      duration: 0.3,
      bounce: 0,
    });
  };

  const handlePressStart = () => {
    if (isCompleted || disabled) return;
    wasArmedRef.current = isArmed;
    clearHoldTimer();
    startedAtRef.current = performance.now();
    setIsHolding(true);
    buttonControls.start({
      transform: shouldReduceMotion
        ? "translateX(0) scale(1)"
        : "translateX(0) scale(0.97)",
      transition: { duration: 0.12, ease: [0.23, 1, 0.32, 1] },
    });
    if (shouldReduceMotion) {
      // Reduced motion: no animated fill. The armed/holding treatment is the
      // only feedback; `commit()` sets the fill to 1 once the timer below
      // actually fires, so the button never looks finished before it is.
      progress.set(0);
    } else {
      animate(progress, 1, {
        duration: resolvedHoldDuration / 1000,
        ease: "linear",
      });
    }
    holdTimerRef.current = setTimeout(() => {
      holdTimerRef.current = null;
      commit();
    }, resolvedHoldDuration);
  };

  /**
   * A genuine release on the button itself: either a full hold already
   * committed via the timer above, a discrete click (arm/commit), or an
   * abandoned hold attempt (cancel).
   */
  const handlePressEnd = () => {
    if (!holdTimerRef.current) return;
    const elapsedMs = performance.now() - startedAtRef.current;
    clearHoldTimer();
    setIsHolding(false);

    if (elapsedMs < CLICK_THRESHOLD_MS) {
      animate(progress, 0, {
        type: "spring",
        duration: 0.3,
        bounce: 0,
      });
      buttonControls.start({ transform: "translateX(0) scale(1)" });
      if (wasArmedRef.current) {
        commit();
      } else {
        arm();
      }
      return;
    }

    disarm();
    runCancelAnimation(elapsedMs);
  };

  // biome-ignore lint/correctness/useExhaustiveDependencies: successDuration is stable per render
  useEffect(() => {
    if (!isCompleted) return;
    successTimerRef.current = setTimeout(resetState, resolvedSuccessDuration);
    return () => {
      if (successTimerRef.current) clearTimeout(successTimerRef.current);
    };
  }, [isCompleted, resolvedSuccessDuration]);

  // Belt-and-braces cancellation for a hold in progress: the keyup/pointerup
  // that would normally cancel it can land elsewhere (global hotkey steals
  // focus, Cmd-Tab away, app backgrounded, pointer released outside the
  // window) and never reach this element's own handlers.
  // biome-ignore lint/correctness/useExhaustiveDependencies: forceCancelHold reads refs/props via stable closures re-created when isHolding flips
  useEffect(() => {
    if (!isHolding) return;
    const handleWindowBlur = () => forceCancelHold();
    const handleVisibilityChange = () => {
      if (document.hidden) forceCancelHold();
    };
    const handleWindowPointerUp = () => forceCancelHold();
    const handleWindowKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") forceCancelHold();
    };
    window.addEventListener("blur", handleWindowBlur);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    window.addEventListener("pointerup", handleWindowPointerUp);
    window.addEventListener("keydown", handleWindowKeyDown);
    return () => {
      window.removeEventListener("blur", handleWindowBlur);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      window.removeEventListener("pointerup", handleWindowPointerUp);
      window.removeEventListener("keydown", handleWindowKeyDown);
    };
  }, [isHolding]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: unmount-only cleanup
  useEffect(
    () => () => {
      clearHoldTimer();
      clearConfirmTimer();
      if (successTimerRef.current) clearTimeout(successTimerRef.current);
    },
    [],
  );

  const statusLabel = isCompleted
    ? "Deleted"
    : isHolding
      ? "Keep holding"
      : isArmed
        ? resolvedConfirmLabel
        : label;

  const renderVisualContent = () => (
    <>
      <span className="relative grid size-4 shrink-0 place-items-center">
        {isCompleted ? (
          <Check aria-hidden="true" className="size-4" strokeWidth={2.25} />
        ) : (
          <Trash2 aria-hidden="true" className="size-4" strokeWidth={2} />
        )}
      </span>
      <span
        aria-hidden="true"
        className="relative grid text-sm leading-none [&>*]:col-start-1 [&>*]:row-start-1"
      >
        <span
          className={cn(
            !isHolding && !isCompleted && !isArmed
              ? "opacity-100"
              : "opacity-0",
          )}
        >
          {label}
        </span>
        <span
          className={cn(
            isHolding && !isCompleted ? "opacity-100" : "opacity-0",
          )}
        >
          Keep holding
        </span>
        <span
          className={cn(
            isArmed && !isHolding && !isCompleted
              ? "opacity-100"
              : "opacity-0",
          )}
        >
          {resolvedConfirmLabel}
        </span>
        <span className={cn(isCompleted ? "opacity-100" : "opacity-0")}>
          Deleted
        </span>
      </span>
    </>
  );

  return (
    <motion.button
      type="button"
      className={cn(
        "relative flex h-12 min-w-48 touch-none cursor-pointer select-none items-center justify-center gap-2 overflow-clip rounded-full px-5 font-medium text-danger shadow-[0_1px_2px_color-mix(in_oklab,var(--color-on-surface)_6%,transparent)] outline-none transition-[background-color,color,box-shadow] duration-150 hover:bg-danger/10 focus-visible:ring-2 focus-visible:ring-danger/50 disabled:cursor-not-allowed disabled:opacity-50",
        isArmed && !isCompleted && "bg-danger/10",
        isCompleted &&
          "bg-success/10 text-success shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--color-success)_40%,transparent),0_1px_2px_color-mix(in_oklab,var(--color-on-surface)_6%,transparent)] focus-visible:ring-success/50",
        className,
      )}
      disabled={disabled}
      aria-busy={isHolding}
      aria-label={label}
      animate={buttonControls}
      onPointerDown={(e) => {
        if (disabled) return;
        // Capture the pointer so a small drift off the row (still holding)
        // doesn't fire pointerleave and cancel the hold: per the Pointer
        // Events spec, boundary events (over/enter/out/leave) don't fire for
        // a captured pointer, so pointermove/pointerup keep targeting this
        // button until release. This also makes onLostPointerCapture below
        // reachable for genuine involuntary interruptions.
        e.currentTarget.setPointerCapture?.(e.pointerId);
        handlePressStart();
      }}
      onPointerUp={(e) => {
        handlePressEnd();
        e.currentTarget.releasePointerCapture?.(e.pointerId);
      }}
      onPointerLeave={forceCancelHold}
      onPointerCancel={(e) => {
        forceCancelHold();
        e.currentTarget.releasePointerCapture?.(e.pointerId);
      }}
      onLostPointerCapture={forceCancelHold}
      onBlur={handleBlur}
      onKeyDown={(e) => {
        if ((e.key === " " || e.key === "Enter") && !e.repeat) {
          e.preventDefault();
          handlePressStart();
        }
      }}
      onKeyUp={(e) => {
        if (e.key === " " || e.key === "Enter") handlePressEnd();
      }}
    >
      <span className="relative flex items-center justify-center gap-2">
        {renderVisualContent()}
      </span>
      <motion.span
        aria-hidden="true"
        className={cn(
          "absolute inset-0 flex items-center gap-2 bg-danger text-danger-foreground [justify-content:inherit] [padding-inline:inherit]",
          isCompleted && "bg-success",
        )}
        style={{ clipPath: progressClipPath }}
      >
        {renderVisualContent()}
      </motion.span>
      <span aria-live="polite" className="sr-only">
        {statusLabel}
      </span>
    </motion.button>
  );
}
