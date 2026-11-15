import { useCallback, useEffect, useRef, useState } from 'react';
import {
  getMainThreadWatchStatus,
  setMainThreadWatchArmed,
  type MainThreadWatchStatus,
} from '../../lib/mainThreadWatch';
import { Toggle } from '../settings/SettingToggle';

const LABEL = 'Main-thread watch';
const SAVE_ERROR = 'Murmur could not change the main-thread watch.';

/**
 * The one switch for the main-thread watch: run-loop observers, a reporter
 * thread and stack sampling that explain slow pastes. It is off until asked
 * for, and takes effect immediately in both directions.
 */
export function MainThreadWatchControl({ active = true }: { active?: boolean }) {
  const [status, setStatus] = useState<MainThreadWatchStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    getMainThreadWatchStatus()
      .then(next => { if (!cancelled) setStatus(next); })
      .catch(() => { if (!cancelled) setError('Murmur could not read the main-thread watch setting.'); });
    return () => { cancelled = true; };
  }, [active]);

  const toggle = useCallback(async () => {
    if (!status || saving) return;
    setSaving(true);
    setError(null);
    try {
      const next = await setMainThreadWatchArmed(!status.setting);
      if (mountedRef.current) setStatus(next);
    } catch {
      if (mountedRef.current) setError(SAVE_ERROR);
    } finally {
      if (mountedRef.current) setSaving(false);
    }
  }, [saving, status]);

  const running = status?.running ?? false;
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
          checked={running}
          onChange={() => { void toggle(); }}
          disabled={!status || saving || status.forcedByEnv}
        />
      </div>
      {status?.forcedByEnv && (
        <p className="mt-1 text-[length:var(--ui-font-caption)] text-on-surface-variant">
          Turned on by the MURMUR_MAIN_THREAD_WATCH environment variable for this launch.
        </p>
      )}
      {error && (
        <p role="alert" className="mt-1 text-[length:var(--ui-font-caption)] text-error">{error}</p>
      )}
    </div>
  );
}
