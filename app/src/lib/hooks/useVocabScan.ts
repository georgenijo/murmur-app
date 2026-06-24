import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { VocabScanSummary } from '../settings';

/**
 * Live progress payload streamed during the walk. Shape matches the Rust
 * `vocab-scan-progress` event exactly (serde camelCase).
 */
export interface VocabScanProgress {
  currentPath: string;
  filesRead: number;
  dirsSkipped: number;
  termsSoFar: number;
  done: boolean;
}

export type VocabScanStatus = 'idle' | 'scanning' | 'done' | 'empty';

/** One streamed row in the live walker tree. */
export interface WalkerRow {
  /** Stable key for React lists (monotonic counter). */
  id: number;
  /**
   * file = read for terms, skip = dependency/build dir skipped. The backend only
   * fires on_file and on_skip — there is no "descended into a dir" callback — so
   * every progress tick is one or the other; there is no 'dir' kind.
   */
  kind: 'file' | 'skip';
  /** Display path (basename-ish — whatever the backend reports as currentPath). */
  path: string;
}

/** Running counts surfaced while scanning + the final summary when done. */
export interface VocabScanStats {
  filesRead: number;
  dirsSkipped: number;
  termsSoFar: number;
  /** Final command result; null until a scan completes. */
  summary: VocabScanSummary | null;
}

const EVENT = 'vocab-scan-progress';
const COMMAND = 'scan_code_vocab';
/** Cap the in-memory walker so a huge repo can't grow the list unbounded. */
const MAX_WALKER_ROWS = 200;

const EMPTY_STATS: VocabScanStats = {
  filesRead: 0,
  dirsSkipped: 0,
  termsSoFar: 0,
  summary: null,
};

export interface UseVocabScan {
  status: VocabScanStatus;
  walker: WalkerRow[];
  stats: VocabScanStats;
  /** Start scanning `folder`. Resolves to the Summary (also surfaced via stats). */
  scan: (folder: string) => Promise<VocabScanSummary | null>;
  /** Stop listening and reset to idle (the backend walk runs to completion regardless). */
  cancel: () => void;
}

/**
 * Drives a code-vocabulary scan: invokes `scan_code_vocab`, subscribes to the
 * throttled `vocab-scan-progress` stream, accumulates walker rows (capped in
 * memory), and resolves to the Summary — `'done'` normally, `'empty'` when no
 * terms were found.
 *
 * Cleanup is the load-bearing part. The active listener is torn down (a) on
 * unmount, (b) at the start of every new scan (no double-subscribe), and
 * (c) when the scan settles or `cancel()` is called (no leak). A run id guards
 * against a stale scan's listener or promise mutating state after a newer scan
 * has started.
 *
 * `initial` seeds the done-state from a persisted summary so the panel shows the
 * last scan immediately on reopen, without re-walking.
 */
export function useVocabScan(initial?: VocabScanSummary | null): UseVocabScan {
  const [status, setStatus] = useState<VocabScanStatus>(() =>
    initial ? (initial.terms > 0 ? 'done' : 'empty') : 'idle',
  );
  const [walker, setWalker] = useState<WalkerRow[]>([]);
  const [stats, setStats] = useState<VocabScanStats>(() =>
    initial
      ? {
          filesRead: initial.files,
          dirsSkipped: initial.skipped,
          termsSoFar: initial.terms,
          summary: initial,
        }
      : EMPTY_STATS,
  );

  // Active event unlisten fn + the run id it belongs to. Refs (not state) so the
  // async scan() closure always sees the current listener without re-running.
  const unlistenRef = useRef<UnlistenFn | null>(null);
  const runIdRef = useRef(0);
  const rowIdRef = useRef(0);
  const mountedRef = useRef(true);
  // Previous tick's CUMULATIVE backend counts, used to classify each new row as
  // file vs. skip by delta. Tracked in refs (not re-derived from the rendered
  // rows) so classification is O(1) and correct even after rows scroll past the
  // MAX_WALKER_ROWS window — re-deriving from the truncated list misreported
  // skips on large repos and misclassified file reads as skips.
  const prevFilesRef = useRef(0);
  const prevSkipsRef = useRef(0);

  // Detach the active progress listener (idempotent). The pending listen()
  // promise (if any) is handled by run-id guards in scan().
  const detach = useCallback(() => {
    if (unlistenRef.current) {
      unlistenRef.current();
      unlistenRef.current = null;
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      detach();
    };
  }, [detach]);

  const cancel = useCallback(() => {
    // Bump the run id so any in-flight listen()/invoke from the cancelled scan
    // is ignored, then detach and reset visible state.
    runIdRef.current += 1;
    detach();
    prevFilesRef.current = 0;
    prevSkipsRef.current = 0;
    if (!mountedRef.current) return;
    setStatus('idle');
    setWalker([]);
    setStats(EMPTY_STATS);
  }, [detach]);

  const scan = useCallback(
    async (folder: string): Promise<VocabScanSummary | null> => {
      // New run: invalidate any prior scan and tear down its listener so we
      // never double-subscribe to the progress stream.
      const runId = runIdRef.current + 1;
      runIdRef.current = runId;
      detach();
      prevFilesRef.current = 0;
      prevSkipsRef.current = 0;

      if (mountedRef.current) {
        setStatus('scanning');
        setWalker([]);
        setStats(EMPTY_STATS);
      }

      // Subscribe before invoking so we don't miss early progress events.
      const unlisten = await listen<VocabScanProgress>(EVENT, (event) => {
        // Stale listener from a superseded scan — ignore.
        if (runId !== runIdRef.current || !mountedRef.current) return;
        const p = event.payload;

        // Terminal event: the backend's final tick carries the real cumulative
        // counts and an empty path. Settle the running counters + terminal status
        // deterministically from it, rather than racing the invoke() resolution.
        // invoke's Summary remains the authoritative stats source (bytes, ms,
        // sampleTerms, capped) and overwrites these counts when it lands.
        if (p.done) {
          setStats((prev) => ({
            ...prev,
            filesRead: p.filesRead,
            dirsSkipped: p.dirsSkipped,
            termsSoFar: p.termsSoFar,
          }));
          setStatus(p.termsSoFar > 0 ? 'done' : 'empty');
          return;
        }

        setStats((prev) => ({
          ...prev,
          filesRead: p.filesRead,
          dirsSkipped: p.dirsSkipped,
          termsSoFar: p.termsSoFar,
        }));

        // The backend reports the most-recent path + cumulative counts. Classify
        // the row by the delta of those cumulative counts against the previous
        // tick (tracked in refs) so it's O(1) and independent of the truncated
        // row window. A skip event bumps dirsSkipped; otherwise it's a file read.
        const kind: WalkerRow['kind'] =
          p.dirsSkipped > prevSkipsRef.current ? 'skip' : 'file';
        prevFilesRef.current = p.filesRead;
        prevSkipsRef.current = p.dirsSkipped;
        setWalker((prev) => {
          const row: WalkerRow = { id: rowIdRef.current++, kind, path: p.currentPath };
          const next = [...prev, row];
          return next.length > MAX_WALKER_ROWS ? next.slice(-MAX_WALKER_ROWS) : next;
        });
      });

      // If a newer scan started (or we unmounted) while awaiting listen(), drop
      // this listener immediately rather than leaking it.
      if (runId !== runIdRef.current || !mountedRef.current) {
        unlisten();
        return null;
      }
      unlistenRef.current = unlisten;

      try {
        const summary = await invoke<VocabScanSummary>(COMMAND, { folder });
        // Superseded or unmounted mid-walk — discard the result. Only tear down a
        // listener that still belongs to THIS run: a newer scan has already
        // installed its own listener into unlistenRef, and the shared detach()
        // would unsubscribe that active newer walk (freezing its live feedback).
        if (runId !== runIdRef.current || !mountedRef.current) {
          if (runId === runIdRef.current) detach();
          return null;
        }
        detach();
        setStats({
          filesRead: summary.files,
          dirsSkipped: summary.skipped,
          termsSoFar: summary.terms,
          summary,
        });
        setStatus(summary.terms > 0 ? 'done' : 'empty');
        return summary;
      } catch (err) {
        if (runId !== runIdRef.current || !mountedRef.current) {
          if (runId === runIdRef.current) detach();
          return null;
        }
        detach();
        if (import.meta.env.DEV) console.debug('[useVocabScan]', err);
        // Treat a failed scan as an empty result so the UI leaves the spinner.
        setStatus('empty');
        return null;
      }
    },
    [detach],
  );

  return { status, walker, stats, scan, cancel };
}
