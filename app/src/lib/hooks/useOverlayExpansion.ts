import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { cursorPosition, getCurrentWindow } from '@tauri-apps/api/window';
import { flog } from '../log';
import {
  COLLAPSE_DELAY_MS,
  HOVER_OPEN_DWELL_MS,
  SHRINK_DELAY_MS,
} from '../overlayMotion';

/**
 * The overlay expand/collapse lifecycle, owned end to end by this controller.
 *
 * After this hook, nothing else in the overlay may call `set_overlay_surface` or
 * own the dwell / collapse / shrink timers. The controller is the single writer
 * to the native resize path, so the CSS reveal and the window resize can never
 * race (problem the PR fixes): opening enqueues the grow, waits for the Rust ack,
 * and only then reveals the dropdown.
 *
 * Phase model:
 *  - `collapsed` — pill only, window at collapsed height.
 *  - `opening`   — grow requested, awaiting the resize ack. CSS not yet revealed.
 *  - `open`      — ack received; the dropdown is revealed. Spans the leave-delay.
 *  - `closing`   — dropdown hidden immediately; window stays tall until the close
 *                  animation finishes, then shrinks back to collapsed.
 */
export type OverlayPhase = 'collapsed' | 'opening' | 'open' | 'closing';

// Poller constants (cursor bounds detection). Not motion tokens: these tune the
// native cursor-tracking safety net, not any visible transition.
const HOVER_POLL_MS = 150;
const HOVER_BOUNDS_PADDING = 8;

interface AppliedSurface {
  windowW: number;
  windowH: number;
}

interface UseOverlayExpansionArgs {
  /** Whether the below-notch preview row is currently visible. */
  previewRowVisible: boolean;
  /** Global-disable state — the cursor poller is gated off while disabled. */
  disabled: boolean;
}

export interface OverlayExpansion {
  /** Current lifecycle phase. Drives re-renders. */
  phase: OverlayPhase;
  /** CSS reveal flag — true only once the grow ack has landed (`phase === 'open'`). */
  expanded: boolean;
  /**
   * Ref that is true while the card is opening or open. Async handlers (e.g. the
   * double-click guard) read this instead of the `expanded` snapshot so a click
   * mid-open is still treated as "card is up".
   */
  expandedRef: React.MutableRefObject<boolean>;
  /** Attach to the visible island element — the poller measures its bounds. */
  islandRef: React.MutableRefObject<HTMLDivElement | null>;
  /** Hover intent started (DOM enter/move or poller entry). Arms the dwell timer. */
  onHoverStart: () => void;
  /** Hover intent ended (DOM leave). Cancels dwell and schedules the close. */
  onHoverEnd: () => void;
}

// A superseded surface request never touches the window. Sentinel so callers can
// distinguish "the newer request won" from a real applied frame.
const SUPERSEDED = Symbol('overlay-surface-superseded');

export function useOverlayExpansion({
  previewRowVisible,
  disabled,
}: UseOverlayExpansionArgs): OverlayExpansion {
  const [phase, setPhase] = useState<OverlayPhase>('collapsed');

  const phaseRef = useRef<OverlayPhase>('collapsed');
  const expandedRef = useRef(false);
  const islandRef = useRef<HTMLDivElement | null>(null);
  const mountedRef = useRef(true);

  // Timers.
  const dwellTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const collapseTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const shrinkTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Serialized surface writer state.
  const genRef = useRef(0);
  const chainRef = useRef<Promise<unknown>>(Promise.resolve());
  const desiredRef = useRef<{ expanded: boolean; previewVisible: boolean }>({
    expanded: false,
    previewVisible: false,
  });
  // Whether the window is currently meant to be at expanded height. Distinct from
  // the CSS `expanded` flag: stays true through `closing` until the shrink lands.
  const windowExpandedRef = useRef(false);

  // Inputs mirrored into refs for the always-on poller / async callbacks.
  const previewRef = useRef(previewRowVisible);
  const disabledRef = useRef(disabled);
  const visibleRef = useRef(true); // default visible on mount, matches show_overlay ordering
  const pollInFlightRef = useRef(false);

  useEffect(() => { disabledRef.current = disabled; }, [disabled]);

  const setPhaseSync = useCallback((next: OverlayPhase) => {
    phaseRef.current = next;
    expandedRef.current = next === 'opening' || next === 'open';
    setPhase(next);
  }, []);

  // --- Phase reconciliation, driven by the surface writer's acks ------------

  const reconcileOnSuccess = useCallback((appliedExpanded: boolean) => {
    if (!mountedRef.current) return;
    if (appliedExpanded && phaseRef.current === 'opening') {
      // The window has actually grown — now it is safe to reveal the dropdown.
      setPhaseSync('open');
    } else if (!appliedExpanded && phaseRef.current === 'closing') {
      setPhaseSync('collapsed');
    }
  }, [setPhaseSync]);

  const reconcileOnFailure = useCallback((appliedExpanded: boolean) => {
    if (!mountedRef.current) return;
    // A failed grow must not reveal; a failed shrink still settles collapsed since
    // the CSS has already hidden the dropdown.
    if (appliedExpanded && phaseRef.current === 'opening') {
      setPhaseSync('collapsed');
    } else if (!appliedExpanded && phaseRef.current === 'closing') {
      setPhaseSync('collapsed');
    }
  }, [setPhaseSync]);

  // --- Serialized surface writer --------------------------------------------

  const applyIfLatest = useCallback(async (gen: number): Promise<AppliedSurface | typeof SUPERSEDED> => {
    if (!mountedRef.current) return SUPERSEDED; // no post-unmount IPC
    if (gen !== genRef.current) return SUPERSEDED; // superseded while queued — skip resize
    const desired = desiredRef.current;
    try {
      const applied = await invoke<AppliedSurface>('set_overlay_surface', {
        expanded: desired.expanded,
        previewVisible: desired.previewVisible,
      });
      if (gen !== genRef.current) return SUPERSEDED; // a newer request will reconcile
      reconcileOnSuccess(desired.expanded);
      return applied;
    } catch (err) {
      if (gen !== genRef.current) return SUPERSEDED; // stale failure — ignore
      flog.warn('overlay', 'set_overlay_surface failed', {
        expanded: desired.expanded,
        previewVisible: desired.previewVisible,
        error: String(err),
      });
      reconcileOnFailure(desired.expanded);
      return SUPERSEDED;
    }
  }, [reconcileOnSuccess, reconcileOnFailure]);

  // Enqueue a resize to the latest desired surface. A newer request supersedes any
  // queued or in-flight older one; stale acks are dropped by the generation guard.
  const pushSurface = useCallback((expanded: boolean) => {
    desiredRef.current = { expanded, previewVisible: previewRef.current };
    windowExpandedRef.current = expanded;
    const gen = ++genRef.current;
    chainRef.current = chainRef.current.then(
      () => applyIfLatest(gen),
      () => applyIfLatest(gen),
    );
  }, [applyIfLatest]);

  // --- Timer helpers --------------------------------------------------------

  const clearOpenDwell = useCallback(() => {
    if (dwellTimerRef.current) { clearTimeout(dwellTimerRef.current); dwellTimerRef.current = null; }
  }, []);

  const clearCloseTimers = useCallback(() => {
    if (collapseTimerRef.current) { clearTimeout(collapseTimerRef.current); collapseTimerRef.current = null; }
    if (shrinkTimerRef.current) { clearTimeout(shrinkTimerRef.current); shrinkTimerRef.current = null; }
  }, []);

  const clearAllTimers = useCallback(() => {
    clearOpenDwell();
    clearCloseTimers();
  }, [clearOpenDwell, clearCloseTimers]);

  // --- Open / close transitions ---------------------------------------------

  // Grow the window first, then reveal the card once the resize is acknowledged.
  const open = useCallback(() => {
    const ph = phaseRef.current;
    if (ph === 'opening' || ph === 'open') return;
    clearCloseTimers();
    setPhaseSync('opening');
    pushSurface(true); // reveal happens in reconcileOnSuccess when the ack lands
  }, [clearCloseTimers, setPhaseSync, pushSurface]);

  // Hide the dropdown immediately, then shrink the window after the close
  // animation so it is never clipped mid-transition.
  const startClosing = useCallback(() => {
    if (phaseRef.current === 'collapsed') return; // nothing to close
    if (phaseRef.current !== 'closing') setPhaseSync('closing');
    if (shrinkTimerRef.current) clearTimeout(shrinkTimerRef.current);
    shrinkTimerRef.current = setTimeout(() => {
      shrinkTimerRef.current = null;
      pushSurface(false); // collapse; reconcileOnSuccess settles to `collapsed`
    }, SHRINK_DELAY_MS);
  }, [setPhaseSync, pushSurface]);

  const beginClose = useCallback((delayMs: number) => {
    clearOpenDwell();
    if (collapseTimerRef.current) clearTimeout(collapseTimerRef.current);
    collapseTimerRef.current = setTimeout(() => {
      collapseTimerRef.current = null;
      startClosing();
    }, delayMs);
  }, [clearOpenDwell, startClosing]);

  // --- Hover intent ---------------------------------------------------------

  // Opening requires hover intent: the cursor must dwell before the card expands,
  // so grazing the notch no longer pops the dropdown. Also cancels any pending
  // close/shrink so re-entry keeps the card up (or reopens cleanly while closing).
  const onHoverStart = useCallback(() => {
    if (collapseTimerRef.current) { clearTimeout(collapseTimerRef.current); collapseTimerRef.current = null; }
    if (shrinkTimerRef.current) { clearTimeout(shrinkTimerRef.current); shrinkTimerRef.current = null; }
    const ph = phaseRef.current;
    if (ph === 'open' || ph === 'opening') return;
    if (dwellTimerRef.current) return;
    dwellTimerRef.current = setTimeout(() => {
      dwellTimerRef.current = null;
      open();
    }, HOVER_OPEN_DWELL_MS);
  }, [open]);

  const onHoverEnd = useCallback(() => {
    clearOpenDwell();
    beginClose(COLLAPSE_DELAY_MS);
  }, [clearOpenDwell, beginClose]);

  // --- Preview-row surface sync ---------------------------------------------
  // The preview row height also flows through the single writer, so it can never
  // race the hover resize. Runs on mount and whenever preview visibility changes.
  useEffect(() => {
    previewRef.current = previewRowVisible;
    pushSurface(windowExpandedRef.current);
  }, [previewRowVisible, pushSurface]);

  // --- Visibility gating for the poller -------------------------------------
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<boolean>('overlay-visible-changed', (event) => {
      visibleRef.current = Boolean(event.payload);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // --- Display-change reset -------------------------------------------------
  // Rust repositions and resizes the overlay to collapsed on a display change, so
  // this listener is authoritative: cancel timers and force `collapsed`. It then
  // issues a corrective collapse THROUGH the writer rather than merely dropping
  // the generation. A grow invoke already dispatched could otherwise be applied
  // *after* Rust's reposition and re-grow the window, leaving a transparent
  // expanded window (and a click dead-zone) until the next hover. pushSurface
  // supersedes that straggler (dropping its ack) and serializes one idempotent
  // collapse resize behind it, repairing the window state.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen('overlay-geometry-changed', () => {
      clearAllTimers();
      setPhaseSync('collapsed');
      pushSurface(false);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [clearAllTimers, setPhaseSync, pushSurface]);

  // --- Single cursor poller -------------------------------------------------
  // The overlay is non-activating and sits above the menu bar, so macOS can miss
  // DOM hover events. One interval branches on phase: strict entry bounds + dwell
  // while collapsed/closing, padded exit bounds while open. Ticks do no IPC while
  // the overlay is hidden, and skip the collapsed entry detector while disabled.
  useEffect(() => {
    const currentWindow = getCurrentWindow();
    const intervalId = setInterval(async () => {
      if (!mountedRef.current) return;
      if (!visibleRef.current) return; // hidden: no IPC
      const ph = phaseRef.current;
      // Disabled gates ONLY the collapsed entry detector (battery). The exit
      // watchdog must stay alive during an active interaction: with the dropdown
      // open, a missed DOM mouseleave — the exact failure this poller exists for —
      // would otherwise leave the card stuck open after the user clicks Disable.
      if (disabledRef.current && ph === 'collapsed') return;
      const island = islandRef.current;
      if (!island) return;
      if (ph === 'opening') return; // transient; nothing to poll
      if (pollInFlightRef.current) return;
      pollInFlightRef.current = true;
      try {
        const [windowPosition, cursor] = await Promise.all([
          currentWindow.outerPosition(),
          cursorPosition(),
        ]);
        const scale = window.devicePixelRatio || 1;
        const rect = island.getBoundingClientRect();

        if (ph === 'open') {
          // Padded exit bounds: collapse once the cursor leaves the visible card.
          const padding = HOVER_BOUNDS_PADDING * scale;
          const left = windowPosition.x + rect.left * scale - padding;
          const right = windowPosition.x + rect.right * scale + padding;
          const top = windowPosition.y + rect.top * scale - padding;
          const bottom = windowPosition.y + rect.bottom * scale + padding;
          if (cursor.x < left || cursor.x > right || cursor.y < top || cursor.y > bottom) {
            beginClose(0);
          }
        } else {
          // Strict entry bounds (collapsed / closing): arm the dwell on hover.
          const left = windowPosition.x + rect.left * scale;
          const right = windowPosition.x + rect.right * scale;
          const top = windowPosition.y + rect.top * scale;
          const bottom = windowPosition.y + rect.bottom * scale;
          if (cursor.x >= left && cursor.x <= right && cursor.y >= top && cursor.y <= bottom) {
            onHoverStart();
          } else {
            clearOpenDwell();
          }
        }
      } catch (err) {
        flog.warn('overlay', 'hover poll failed', { error: String(err) });
      } finally {
        pollInFlightRef.current = false;
      }
    }, HOVER_POLL_MS);
    return () => clearInterval(intervalId);
  }, [beginClose, onHoverStart, clearOpenDwell]);

  // --- Mount / unmount lifecycle --------------------------------------------
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      clearAllTimers();
    };
  }, [clearAllTimers]);

  return {
    phase,
    expanded: phase === 'open',
    expandedRef,
    islandRef,
    onHoverStart,
    onHoverEnd,
  };
}
