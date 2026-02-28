import { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { isDictationStatus } from '../lib/types';
import type { DictationStatus } from '../lib/types';
import { flog } from '../lib/log';
import { STORAGE_KEY, DEFAULT_SETTINGS } from '../lib/settings';

const BAR_COUNT = 7;

export function OverlayWidget() {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const [lockedMode, setLockedMode] = useState(false);
  const notchHeightRef = useRef(0);
  const [notchWidth, setNotchWidth] = useState(185);
  const statusRef = useRef(status);
  const lockedRef = useRef(lockedMode);
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const audioLevelRef = useRef(0);
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);

  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { lockedRef.current = lockedMode; }, [lockedMode]);

  // Log mount + fetch notch dimensions
  useEffect(() => {
    flog.info('overlay', 'mounted');
    invoke<{ notch_width: number; notch_height: number } | null>('get_notch_info')
      .then((info) => {
        if (info) {
          flog.info('overlay', 'notch info', { notch_width: info.notch_width, notch_height: info.notch_height });
          notchHeightRef.current = info.notch_height;
          setNotchWidth(info.notch_width);
        } else {
          flog.info('overlay', 'no notch detected');
        }
      })
      .catch((e) => flog.warn('overlay', 'get_notch_info failed', { error: String(e) }));
    return () => {
      flog.info('overlay', 'unmounted');
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

  // TODO: re-enable position persistence after notch positioning is stable.
  // Both save (onMoved) and restore are disabled to avoid saving programmatic repositions.

  // Subscribe to recording status events from Rust
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<string>('recording-status-changed', (event) => {
      if (isDictationStatus(event.payload)) {
        flog.info('overlay', 'status changed', { status: event.payload });
        setStatus(event.payload);
        if (event.payload === 'idle') {
          setLockedMode(false);
        }
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Subscribe to notch info changes (display config change: monitor plug/unplug, lid)
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<{ notch_width: number; notch_height: number } | null>('notch-info-changed', (event) => {
      if (event.payload) {
        flog.info('overlay', 'notch info changed', { notch_width: event.payload.notch_width, notch_height: event.payload.notch_height });
        notchHeightRef.current = event.payload.notch_height;
        setNotchWidth(event.payload.notch_width);
      } else {
        flog.info('overlay', 'notch removed (no notch on new display)');
        notchHeightRef.current = 0;
        setNotchWidth(185);
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Subscribe to audio level events from Rust (store in ref, no state update)
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<number>('audio-level', (event) => {
      audioLevelRef.current = event.payload;
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Animate waveform bars via rAF (direct DOM updates, no React reconciliation)
  useEffect(() => {
    if (status !== 'recording') {
      barRefs.current.forEach(el => {
        if (el) el.style.height = '2px';
      });
      return;
    }
    let rafId: number;
    const animate = () => {
      const level = Math.min(1, audioLevelRef.current * 16);
      barRefs.current.forEach((el, i) => {
        if (!el) return;
        const baseline = 0.08 + Math.random() * 0.07;
        const center = (BAR_COUNT - 1) / 2;
        const distFromCenter = 1 - Math.abs(i - center) / center;
        const envelope = 0.5 + 0.5 * distFromCenter;
        const reactiveHeight = level * envelope;
        const boost = level * level * 0.4 * Math.random();
        const h = Math.min(1, baseline + reactiveHeight + boost);
        el.style.height = `${Math.max(2, Math.round(h * 14))}px`;
      });
      rafId = requestAnimationFrame(animate);
    };
    rafId = requestAnimationFrame(animate);
    return () => cancelAnimationFrame(rafId);
  }, [status]);

  // Double-click: toggle locked mode. Cancel any pending single-click first.
  const handleDoubleClick = useCallback(async (e: React.MouseEvent) => {
    flog.info('overlay', 'DOUBLE-CLICK', {
      locked: lockedRef.current, status: statusRef.current,
      x: Math.round(e.clientX), y: Math.round(e.clientY),
      target: (e.target as HTMLElement).tagName,
    });
    const currentStatus = statusRef.current;
    if (currentStatus === 'processing') return;
    if (clickTimerRef.current) {
      clearTimeout(clickTimerRef.current);
      clickTimerRef.current = null;
    }
    const currentLocked = lockedRef.current;
    if (!currentLocked) {
      // Enter locked mode — start recording
      setLockedMode(true);
      if (currentStatus !== 'recording') {
        try {
          // Read microphone setting from localStorage (overlay has no React settings context)
          let deviceName: string | null = null;
          try {
            const stored = localStorage.getItem(STORAGE_KEY);
            if (stored) {
              const parsed = JSON.parse(stored);
              if (parsed.microphone && parsed.microphone !== DEFAULT_SETTINGS.microphone) {
                deviceName = parsed.microphone;
              }
            }
          } catch { /* ignore parse errors */ }
          flog.info('overlay', 'invoking start_native_recording', { deviceName });
          const res = await invoke('start_native_recording', { deviceName });
          flog.info('overlay', 'start_native_recording result', { res: res as Record<string, unknown> });
        } catch (err) {
          flog.error('overlay', 'start_native_recording error', { error: String(err) });
          setLockedMode(false);
        }
      }
    } else {
      // Exit locked mode — stop recording
      setLockedMode(false);
      if (currentStatus === 'recording') {
        try {
          flog.info('overlay', 'invoking stop_native_recording');
          const res = await invoke('stop_native_recording');
          flog.info('overlay', 'stop_native_recording result', { res: res as Record<string, unknown> });
        } catch (err) {
          flog.error('overlay', 'stop_native_recording error', { error: String(err) });
        }
      }
    }
  }, []);

  // Single-click: debounced so it doesn't fire when the user double-clicks.
  const handleClick = useCallback((e: React.MouseEvent) => {
    if (clickTimerRef.current) {
      clearTimeout(clickTimerRef.current);
    }
    flog.info('overlay', 'CLICK (pending 250ms debounce)', {
      locked: lockedRef.current, status: statusRef.current,
      x: Math.round(e.clientX), y: Math.round(e.clientY),
      target: (e.target as HTMLElement).tagName,
    });
    clickTimerRef.current = setTimeout(async () => {
      clickTimerRef.current = null;
      flog.info('overlay', 'click fired', { locked: lockedRef.current, status: statusRef.current });
      // Single click stops recording (regardless of locked mode)
      if (statusRef.current === 'recording') {
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

  // Raw mousedown — fires before click/double-click debouncing
  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    flog.info('overlay', 'MOUSEDOWN', {
      button: e.button, x: Math.round(e.clientX), y: Math.round(e.clientY),
      target: (e.target as HTMLElement).tagName,
      className: (e.target as HTMLElement).className?.slice(0, 50),
      locked: lockedRef.current, status: statusRef.current,
    });
  }, []);

  return (
    <div
      data-tauri-drag-region
      className="w-full h-full flex"
      style={{ background: 'transparent' }}
      onMouseDown={handleMouseDown}
      onDoubleClick={handleDoubleClick}
      onClick={handleClick}
    >
      {/* Dynamic Island: same height as notch, expands horizontally.
          Idle = small icon on the left showing app is alive.
          Active = expands with red dot on left, waveform on right. */}
      <div
        className="cursor-pointer select-none overflow-hidden transition-all duration-[500ms] ease-[cubic-bezier(0.34,1.56,0.64,1)]"
        style={{
          borderRadius: '0 0 12px 12px',
          width: isActive ? notchWidth + 68 : notchWidth + 28,
          marginLeft: 32,
          height: '100%',
          background: 'rgba(20, 20, 20, 0.92)',
          backdropFilter: 'blur(40px)',
          WebkitBackdropFilter: 'blur(40px)',
        }}
      >
        <div className="flex items-center h-full" style={{ paddingLeft: 10, paddingRight: 10 }}>
          {/* Left side — mic icon (idle) or red dot (recording) or spinner (processing), all same position */}
          <div className="shrink-0 w-3 h-3 flex items-center justify-center">
            {status === 'recording' ? (
              <div className="w-2.5 h-2.5 rounded-full bg-red-500" style={{ animation: 'pulse 0.8s ease-in-out infinite' }} />
            ) : status === 'processing' ? (
              <span className="w-3 h-3 border-[1.5px] border-white/20 border-t-white/70 rounded-full animate-spin block" />
            ) : (
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="rgba(255,255,255,0.4)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <rect x="9" y="1" width="6" height="12" rx="3" />
                <path d="M5 10a7 7 0 0 0 14 0" />
                <line x1="12" y1="17" x2="12" y2="21" />
              </svg>
            )}
          </div>

          {/* Spacer */}
          <div className="flex-1" />

          {/* Right side — waveform (only when active) */}
          <div
            className="flex items-center gap-[1.5px] h-4 shrink-0 transition-opacity duration-300"
            style={{ opacity: isActive ? 1 : 0 }}
          >
            {Array.from({ length: BAR_COUNT }, (_, i) => (
              <div
                key={i}
                ref={el => { barRefs.current[i] = el; }}
                className={`w-[2px] rounded-full ${
                  status === 'recording' ? 'bg-white/90' : 'bg-white/40'
                }`}
                style={{
                  height: '2px',
                  transition: `height ${status === 'recording' ? '50ms' : '300ms'} ease-out`,
                }}
              />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
