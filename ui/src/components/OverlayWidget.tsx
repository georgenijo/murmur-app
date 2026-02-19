import { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow, primaryMonitor, LogicalPosition } from '@tauri-apps/api/window';
import type { DictationStatus } from '../lib/types';

const BAR_COUNT = 5;

const VALID_STATUSES = ['idle', 'recording', 'processing'] as const;
function isDictationStatus(v: unknown): v is DictationStatus {
  return typeof v === 'string' && (VALID_STATUSES as readonly string[]).includes(v);
}

export function OverlayWidget() {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const [audioLevel, setAudioLevel] = useState(0);
  const [lockedMode, setLockedMode] = useState(false);
  const [barHeights, setBarHeights] = useState<number[]>(Array(BAR_COUNT).fill(0.15));
  const statusRef = useRef(status);
  const lockedRef = useRef(lockedMode);

  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { lockedRef.current = lockedMode; }, [lockedMode]);

  // Position window at bottom-center of primary monitor on mount
  useEffect(() => {
    (async () => {
      try {
        const monitor = await primaryMonitor();
        if (!monitor) return;
        const { width: mw, height: mh } = monitor.size;
        const sf = monitor.scaleFactor;
        const overlayW = 200;
        const overlayH = 60;
        const x = Math.round((mw / sf - overlayW) / 2);
        const y = Math.round(mh / sf - overlayH - 50);
        await getCurrentWindow().setPosition(new LogicalPosition(x, y));
      } catch {
        // Best-effort positioning
      }
    })();
  }, []);

  // Subscribe to recording status events from Rust
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<string>('recording-status-changed', (event) => {
      if (isDictationStatus(event.payload)) {
        setStatus(event.payload);
      }
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);

  // Subscribe to audio level events from Rust
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<number>('audio-level', (event) => {
      setAudioLevel(event.payload);
    }).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);

  // Animate waveform bars based on audio level
  useEffect(() => {
    if (status !== 'recording') {
      setBarHeights(Array(BAR_COUNT).fill(0.15));
      return;
    }
    const clampedLevel = Math.min(1, audioLevel * 4); // amplify for visibility
    setBarHeights(
      Array.from({ length: BAR_COUNT }, (_, i) => {
        const phase = (i / BAR_COUNT) * Math.PI * 2;
        const jitter = Math.random() * 0.25;
        return Math.min(1, Math.max(0.1, clampedLevel * (0.6 + 0.4 * Math.sin(phase)) + jitter));
      })
    );
  }, [audioLevel, status]);

  const handleDoubleClick = useCallback(async () => {
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

  const handleClick = useCallback(async () => {
    // In locked mode, a single click stops recording
    if (lockedRef.current && statusRef.current === 'recording') {
      setLockedMode(false);
      try {
        await invoke('stop_native_recording');
      } catch {
        // status will sync via event
      }
    }
  }, []);

  const statusColor = {
    idle: 'bg-stone-700/80',
    recording: 'bg-red-600/90',
    processing: 'bg-amber-500/90',
  }[status];

  const statusLabel = {
    idle: 'Idle',
    recording: lockedMode ? 'Locked' : 'Recording',
    processing: 'Processing…',
  }[status];

  return (
    <div
      className="w-full h-full flex items-center justify-center"
      style={{ background: 'transparent' }}
      onDoubleClick={handleDoubleClick}
      onClick={handleClick}
    >
      <div
        className={`
          flex items-center gap-2 px-4 py-2 rounded-full
          ${statusColor}
          backdrop-blur-sm cursor-pointer select-none
          transition-colors duration-300
          shadow-lg
        `}
        style={{ minWidth: 140 }}
      >
        {/* Waveform bars */}
        <div className="flex items-center gap-[3px] h-5">
          {barHeights.map((h, i) => (
            <div
              key={i}
              className={`w-[3px] rounded-full transition-all ${
                status === 'recording' ? 'bg-white' : 'bg-white/40'
              }`}
              style={{
                height: `${Math.round(h * 20)}px`,
                transitionDuration: status === 'recording' ? '80ms' : '300ms',
              }}
            />
          ))}
        </div>

        {/* Status label */}
        <span className="text-white text-xs font-medium tracking-wide">
          {statusLabel}
        </span>

        {/* Lock indicator */}
        {lockedMode && (
          <span className="text-white/70 text-xs">🔒</span>
        )}

        {/* Processing spinner */}
        {status === 'processing' && (
          <span
            className="w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin"
          />
        )}
      </div>
    </div>
  );
}
