import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emit } from '@tauri-apps/api/event';
import {
  clampOverlayVerticalOffset,
  OVERLAY_VERTICAL_OFFSET_MAX,
  OVERLAY_VERTICAL_OFFSET_MIN,
} from '../../lib/settings';

interface OverlayCalibrationControlProps {
  offset: number;
  onCommit: (offset: number) => void;
}

function offsetLabel(offset: number): string {
  if (offset === 0) return 'Default position';
  return `${offset > 0 ? '+' : ''}${offset} pt`;
}

export function OverlayCalibrationControl({
  offset,
  onCommit,
}: OverlayCalibrationControlProps) {
  const [active, setActive] = useState(false);
  const [draft, setDraft] = useState(() => clampOverlayVerticalOffset(offset));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const activeRef = useRef(false);
  const adjustUpRef = useRef<HTMLButtonElement>(null);
  const startRef = useRef<HTMLButtonElement>(null);
  const focusPendingRef = useRef(false);
  const originalOffsetRef = useRef(clampOverlayVerticalOffset(offset));
  const requestGenerationRef = useRef(0);

  useEffect(() => {
    if (!focusPendingRef.current) return;
    focusPendingRef.current = false;
    (active ? adjustUpRef.current : startRef.current)?.focus();
  }, [active]);

  useEffect(() => {
    if (!activeRef.current) setDraft(clampOverlayVerticalOffset(offset));
  }, [offset]);

  const announceCalibration = useCallback((nextActive: boolean) => {
    void emit('overlay-calibration-changed', { active: nextActive }).catch(() => {});
  }, []);

  const preview = useCallback(async (nextOffset: number): Promise<boolean> => {
    const next = clampOverlayVerticalOffset(nextOffset);
    const generation = ++requestGenerationRef.current;
    setBusy(true);
    setError(null);
    try {
      await invoke('set_overlay_vertical_offset', { offset: next });
      if (generation !== requestGenerationRef.current) return false;
      setDraft(next);
      return true;
    } catch {
      if (generation === requestGenerationRef.current) {
        setError('Murmur could not move the overlay. Try again.');
      }
      return false;
    } finally {
      if (generation === requestGenerationRef.current) setBusy(false);
    }
  }, []);

  const start = useCallback(async () => {
    const original = clampOverlayVerticalOffset(offset);
    const generation = ++requestGenerationRef.current;
    setBusy(true);
    setError(null);
    try {
      await invoke('show_overlay');
      await invoke('set_overlay_vertical_offset', { offset: original });
      if (generation !== requestGenerationRef.current) return;
      originalOffsetRef.current = original;
      setDraft(original);
      activeRef.current = true;
      focusPendingRef.current = true;
      setActive(true);
      announceCalibration(true);
    } catch {
      if (generation === requestGenerationRef.current) {
        setError('Murmur could not start overlay calibration. Try again.');
      }
    } finally {
      if (generation === requestGenerationRef.current) setBusy(false);
    }
  }, [announceCalibration, offset]);

  const finish = useCallback(() => {
    activeRef.current = false;
    focusPendingRef.current = true;
    setActive(false);
    announceCalibration(false);
  }, [announceCalibration]);

  const cancel = useCallback(async () => {
    if (await preview(originalOffsetRef.current)) finish();
  }, [finish, preview]);

  const save = useCallback(() => {
    onCommit(draft);
    originalOffsetRef.current = draft;
    finish();
  }, [draft, finish, onCommit]);

  const previewDefault = useCallback(async () => {
    await preview(0);
  }, [preview]);

  const resetConfirmed = useCallback(async () => {
    if (await preview(0)) onCommit(0);
  }, [onCommit, preview]);

  useEffect(() => () => {
    requestGenerationRef.current += 1;
    if (!activeRef.current) return;
    void invoke('set_overlay_vertical_offset', { offset: originalOffsetRef.current });
    announceCalibration(false);
  }, [announceCalibration]);

  return (
    <div className="rounded-lg border border-outline-variant/30 bg-surface-container-lowest px-3 py-3">
      <div className="flex items-start justify-between gap-4">
        <div>
          <p className="text-xs font-medium text-on-surface">Overlay position</p>
          <p className="mt-1 text-xs text-on-surface-variant">
            Fine-tune the overlay vertically if it does not sit flush with the notch.
          </p>
        </div>
        {!active && (
          <span className="shrink-0 rounded-full bg-surface-container px-2 py-1 text-[11px] tabular-nums text-on-surface-variant">
            {offsetLabel(clampOverlayVerticalOffset(offset))}
          </span>
        )}
      </div>

      {active ? (
        <div className="mt-3 border-t border-outline-variant/20 pt-3">
          <p className="text-xs text-on-surface-variant">
            Adjust the live overlay, then save when it lines up with the notch.
          </p>
          <div className="mt-3 flex items-center justify-center gap-3">
            <button
              ref={adjustUpRef}
              type="button"
              aria-label="Move overlay up one point"
              disabled={busy || draft <= OVERLAY_VERTICAL_OFFSET_MIN}
              onClick={() => void preview(draft - 1)}
              className="grid h-8 w-10 place-items-center rounded-lg border border-outline-variant/30 bg-surface-container text-sm text-on-surface hover:border-primary/50 hover:text-primary disabled:cursor-not-allowed disabled:opacity-35"
            >
              ↑
            </button>
            <output
              aria-live="polite"
              className="min-w-[112px] text-center text-sm font-semibold tabular-nums text-on-surface"
            >
              {offsetLabel(draft)}
            </output>
            <button
              type="button"
              aria-label="Move overlay down one point"
              disabled={busy || draft >= OVERLAY_VERTICAL_OFFSET_MAX}
              onClick={() => void preview(draft + 1)}
              className="grid h-8 w-10 place-items-center rounded-lg border border-outline-variant/30 bg-surface-container text-sm text-on-surface hover:border-primary/50 hover:text-primary disabled:cursor-not-allowed disabled:opacity-35"
            >
              ↓
            </button>
          </div>
          <div className="mt-3 flex items-center justify-between gap-2">
            <button
              type="button"
              disabled={busy || draft === 0}
              onClick={() => void previewDefault()}
              className="rounded-lg px-2 py-1.5 text-xs font-medium text-on-surface-variant hover:bg-surface-container hover:text-primary disabled:cursor-not-allowed disabled:opacity-35"
            >
              Preview default
            </button>
            <div className="flex gap-2">
              <button
                type="button"
                disabled={busy}
                onClick={() => void cancel()}
                className="rounded-lg border border-outline-variant/30 px-3 py-1.5 text-xs font-medium text-on-surface-variant hover:bg-surface-container disabled:cursor-not-allowed disabled:opacity-50"
              >
                Cancel
              </button>
              <button
                type="button"
                disabled={busy}
                onClick={save}
                className="rounded-(--ui-radius-pill) bg-primary shadow-(--ui-shadow-accent) px-3 py-1.5 text-xs font-semibold text-on-primary hover:bg-primary-dim disabled:cursor-not-allowed disabled:opacity-50"
              >
                Save position
              </button>
            </div>
          </div>
        </div>
      ) : (
        <div className="mt-3 flex gap-2">
          <button
            ref={startRef}
            type="button"
            disabled={busy}
            onClick={() => void start()}
            className="flex-1 rounded-lg border border-outline-variant/30 bg-surface-container px-3 py-2 text-xs font-medium text-on-surface-variant transition-colors hover:border-primary/40 hover:text-primary disabled:cursor-not-allowed disabled:opacity-50"
          >
            {busy ? 'Opening overlay…' : 'Calibrate'}
          </button>
          <button
            type="button"
            disabled={busy || clampOverlayVerticalOffset(offset) === 0}
            onClick={() => void resetConfirmed()}
            className="rounded-lg border border-outline-variant/30 px-3 py-2 text-xs font-medium text-on-surface-variant transition-colors hover:bg-surface-container hover:text-primary disabled:cursor-not-allowed disabled:opacity-35"
          >
            Reset
          </button>
        </div>
      )}

      {error && <p role="alert" className="mt-2 text-xs text-error">{error}</p>}
    </div>
  );
}
