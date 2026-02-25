import { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow, PhysicalPosition } from '@tauri-apps/api/window';
import { isDictationStatus } from '../lib/types';
import type { DictationStatus } from '../lib/types';

const BAR_COUNT = 7;
const POSITION_KEY = 'overlay-position';

export function OverlayWidget() {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const [audioLevel, setAudioLevel] = useState(0);
  const [lockedMode, setLockedMode] = useState(false);
  const [barHeights, setBarHeights] = useState<number[]>(Array(BAR_COUNT).fill(0.15));
  const statusRef = useRef(status);
  const lockedRef = useRef(lockedMode);
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { lockedRef.current = lockedMode; }, [lockedMode]);

  // Log mount
  useEffect(() => {
    console.log('[overlay] mounted');
    return () => {
      console.log('[overlay] unmounted');
      if (clickTimerRef.current) clearTimeout(clickTimerRef.current);
    };
  }, []);

  // Restore saved position (Rust handles default positioning)
  // TODO: re-enable after notch positioning is stable
  // useEffect(() => {
  //   (async () => {
  //     try {
  //       const saved = localStorage.getItem(POSITION_KEY);
  //       if (saved) {
  //         const { x, y } = JSON.parse(saved) as { x: number; y: number };
  //         console.log('[overlay] restoring saved position:', { x, y });
  //         await getCurrentWindow().setPosition(new PhysicalPosition(x, y));
  //       } else {
  //         console.log('[overlay] no saved position, using Rust default');
  //       }
  //     } catch (e) {
  //       console.warn('[overlay] position restore failed:', e);
  //     }
  //   })();
  // }, []);

  // Persist overlay position on move (debounced)
  useEffect(() => {
    let debounceTimer: ReturnType<typeof setTimeout> | null = null;
    let cancelled = false;
    let unlisten: (() => void) | null = null;

    getCurrentWindow().onMoved(({ payload }) => {
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(() => {
        if (!cancelled) {
          console.log('[overlay] saving position:', { x: payload.x, y: payload.y });
          try {
            localStorage.setItem(POSITION_KEY, JSON.stringify({
              x: payload.x,
              y: payload.y,
            }));
          } catch (e) {
            console.warn('[overlay] position save failed:', e);
          }
        }
      }, 500);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });

    return () => {
      cancelled = true;
      if (debounceTimer) clearTimeout(debounceTimer);
      unlisten?.();
    };
  }, []);

  // Subscribe to recording status events from Rust
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<string>('recording-status-changed', (event) => {
      if (isDictationStatus(event.payload)) {
        console.log('[overlay] status changed:', event.payload);
        setStatus(event.payload);
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Subscribe to audio level events from Rust
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<number>('audio-level', (event) => {
      setAudioLevel(event.payload);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Animate waveform bars based on audio level
  useEffect(() => {
    if (status !== 'recording') {
      setBarHeights(Array(BAR_COUNT).fill(0.15));
      return;
    }
    const level = Math.min(1, audioLevel * 16); // very aggressive — reacts to whispers
    setBarHeights(
      Array.from({ length: BAR_COUNT }, (_, i) => {
        const baseline = 0.08 + Math.random() * 0.07;
        const center = (BAR_COUNT - 1) / 2;
        const distFromCenter = 1 - Math.abs(i - center) / center;
        const envelope = 0.5 + 0.5 * distFromCenter;
        const reactiveHeight = level * envelope;
        // Square the level to make loud sounds WAY bigger
        const boost = level * level * 0.4 * Math.random();
        return Math.min(1, baseline + reactiveHeight + boost);
      })
    );
  }, [audioLevel, status]);

  // Double-click: toggle locked mode. Cancel any pending single-click first.
  const handleDoubleClick = useCallback(async () => {
    console.log('[overlay] double-click', { locked: lockedRef.current, status: statusRef.current });
    if (clickTimerRef.current) {
      clearTimeout(clickTimerRef.current);
      clickTimerRef.current = null;
    }
    const currentLocked = lockedRef.current;
    const currentStatus = statusRef.current;
    if (!currentLocked) {
      // Enter locked mode — start recording
      setLockedMode(true);
      if (currentStatus !== 'recording') {
        try {
          await invoke('start_native_recording');
        } catch {
          setLockedMode(false);
        }
      }
    } else {
      // Exit locked mode — stop recording
      setLockedMode(false);
      if (currentStatus === 'recording') {
        try {
          await invoke('stop_native_recording');
        } catch {
          // status will sync via recording-status-changed event
        }
      }
    }
  }, []);

  // Single-click: debounced so it doesn't fire when the user double-clicks.
  const handleClick = useCallback(() => {
    console.log('[overlay] click (pending 250ms debounce)', { locked: lockedRef.current, status: statusRef.current });
    clickTimerRef.current = setTimeout(async () => {
      clickTimerRef.current = null;
      console.log('[overlay] click fired', { locked: lockedRef.current, status: statusRef.current });
      // In locked mode, a single click stops recording
      if (lockedRef.current && statusRef.current === 'recording') {
        setLockedMode(false);
        try {
          await invoke('stop_native_recording');
        } catch {
          // status will sync via event
        }
      }
    }, 250);
  }, []);

  const isActive = status === 'recording' || status === 'processing';

  const statusLabel = {
    idle: 'Idle',
    recording: lockedMode ? 'Locked' : 'Recording',
    processing: 'Processing…',
  }[status];

  return (
    <div
      data-tauri-drag-region
      className="w-full h-full flex justify-center"
      style={{ background: 'transparent' }}
      onDoubleClick={handleDoubleClick}
      onClick={handleClick}
    >
      {/* Notch extension: flat top merges with notch, rounded bottom.
          Idle = tiny 4px nub hidden behind notch.
          Active = expands down to reveal content. */}
      <div
        className="bg-black cursor-pointer select-none overflow-hidden transition-all duration-[400ms] ease-[cubic-bezier(0.4,0,0.2,1)]"
        style={{
          borderRadius: '0 0 14px 14px',
          width: isActive ? '100%' : 80,
          height: isActive ? 44 : 4,
        }}
      >
        {/* Content only visible when expanded */}
        <div
          className="flex items-center justify-center gap-2.5 h-full px-4 transition-opacity duration-300"
          style={{ opacity: isActive ? 1 : 0 }}
        >
          {/* Recording dot */}
          {status === 'recording' && (
            <div className="w-2 h-2 rounded-full bg-red-500 animate-pulse shrink-0" />
          )}

          {/* Waveform bars */}
          <div className="flex items-center gap-[2px] h-6">
            {barHeights.map((h, i) => (
              <div
                key={i}
                className={`w-[2.5px] rounded-full ${
                  status === 'recording' ? 'bg-white/90' : 'bg-white/40'
                }`}
                style={{
                  height: `${Math.max(2, Math.round(h * 24))}px`,
                  transition: `height ${status === 'recording' ? '50ms' : '300ms'} ease-out`,
                }}
              />
            ))}
          </div>

          {/* Status label */}
          <span className="text-white/70 text-[11px] font-medium tracking-wide whitespace-nowrap">
            {statusLabel}
          </span>

          {/* Lock indicator */}
          {lockedMode && (
            <span className="text-white/40 text-[10px]">🔒</span>
          )}

          {/* Processing spinner */}
          {status === 'processing' && (
            <span className="w-3 h-3 border-[1.5px] border-white/20 border-t-white/70 rounded-full animate-spin shrink-0" />
          )}
        </div>
      </div>
    </div>
  );
}
