import { useCallback, useEffect, useRef, useState } from 'react';
import {
  getMainThreadWatchStatus,
  listenMainThreadWatchStatus,
  setMainThreadWatchArmed,
  type MainThreadWatchStatus,
} from '../../lib/mainThreadWatch';
import { Toggle } from '../settings/SettingToggle';

const LABEL = 'Main-thread watch';
const RESTART_NOTE = 'Takes effect after you restart Murmur.';
const SAVE_ERROR = 'Murmur could not change the main-thread watch.';

/**
 * The one switch for the main-thread watch: run-loop observers, a reporter
 * thread and stack sampling that explain slow pastes. It is off until asked
 * for. The switch only saves the setting: the watch starts or stops at the
 * next launch, so the control shows the saved setting and, when that differs
 * from what this launch is doing, says a restart is needed.
 */
export function MainThreadWatchControl({ active = true }: { active?: boolean }) {
  const [status, setStatus] = useState<MainThreadWatchStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);
  // The newest status seen, from a read, a change made here, or a change made
  // in another Diagnostics view. Older ones are ignored.
  const seqRef = useRef(-1);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const accept = useCallback((next: MainThreadWatchStatus) => {
    if (!mountedRef.current || next.seq < seqRef.current) return;
    seqRef.current = next.seq;
    setStatus(next);
    setError(null);
  }, []);

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    // Subscribe first, then read: a change made between the two is heard, and
    // a read that resolves after a newer event cannot overwrite it.
    listenMainThreadWatchStatus(accept)
      .then(stop => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => {})
      .then(() => getMainThreadWatchStatus())
      .then(next => { if (!cancelled) accept(next); })
      .catch(() => { if (!cancelled) setError('Murmur could not read the main-thread watch setting.'); });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [accept, active]);

  const toggle = useCallback(async () => {
    if (!status || saving) return;
    setSaving(true);
    setError(null);
    try {
      accept(await setMainThreadWatchArmed(!status.setting));
    } catch {
      if (mountedRef.current) setError(SAVE_ERROR);
    } finally {
      if (mountedRef.current) setSaving(false);
    }
  }, [accept, saving, status]);

  const wanted = status ? status.setting || status.forcedByEnv : false;
  const restartPending = status !== null && wanted !== status.runningThisLaunch;
  return (
    <div
      data-testid="main-thread-watch-control"
      className="rounded-xl border border-outline-variant/15 bg-surface-container-low px-3 py-2"
    >
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-[length:var(--ui-font-body)] font-medium text-on-surface">{LABEL}</div>
          <p className="mt-0.5 text-[length:var(--ui-font-caption)] text-on-surface-variant">
            Explains slow pastes. Uses extra CPU and log volume while on.
          </p>
        </div>
        <Toggle
          label={LABEL}
          checked={status?.setting ?? false}
          onChange={() => { void toggle(); }}
          disabled={!status || saving}
        />
      </div>
      {status?.forcedByEnv && (
        <p className="mt-1 text-[length:var(--ui-font-caption)] text-on-surface-variant">
          Running in this launch because of the MURMUR_MAIN_THREAD_WATCH environment variable.
        </p>
      )}
      {restartPending && (
        <p
          data-testid="main-thread-watch-restart-note"
          className="mt-1 text-[length:var(--ui-font-caption)] text-on-surface-variant"
        >
          {RESTART_NOTE}
        </p>
      )}
      {error && (
        <p role="alert" className="mt-1 text-[length:var(--ui-font-caption)] text-error">{error}</p>
      )}
    </div>
  );
}
