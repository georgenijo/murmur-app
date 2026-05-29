import { useState, useEffect, useCallback, useRef } from 'react';
import { listen, emit } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { isDictationStatus } from '../lib/types';
import type { DictationStatus } from '../lib/types';
import { flog } from '../lib/log';
import { STORAGE_KEY, DEFAULT_SETTINGS, loadSettings, saveSettings } from '../lib/settings';

const BAR_COUNT = 7;

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${String(s).padStart(2, '0')}`;
}

export function OverlayWidget() {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const [showCancelled, setShowCancelled] = useState(false);
  const [lockedMode, setLockedMode] = useState(false);
  const [disabled, setDisabled] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [autoPaste, setAutoPaste] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const notchHeightRef = useRef(0);
  const [notchHeight, setNotchHeight] = useState(0);
  const [notchWidth, setNotchWidth] = useState(185);
  const statusRef = useRef(status);
  const lockedRef = useRef(lockedMode);
  const disabledRef = useRef(disabled);
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const collapseTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const shrinkTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const audioLevelRef = useRef(0);
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);

  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { lockedRef.current = lockedMode; }, [lockedMode]);
  useEffect(() => { disabledRef.current = disabled; }, [disabled]);

  // Log mount + fetch notch dimensions + read initial disabled state
  useEffect(() => {
    flog.info('overlay', 'mounted');
    invoke<{ notch_width: number; notch_height: number } | null>('get_notch_info')
      .then((info) => {
        if (info) {
          flog.info('overlay', 'notch info', { notch_width: info.notch_width, notch_height: info.notch_height });
          notchHeightRef.current = info.notch_height;
          setNotchHeight(info.notch_height);
          setNotchWidth(info.notch_width);
        } else {
          flog.info('overlay', 'no notch detected');
        }
      })
      .catch((e) => flog.warn('overlay', 'get_notch_info failed', { error: String(e) }));
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      if (stored) {
        const parsed = JSON.parse(stored);
        if (typeof parsed.disabled === 'boolean') setDisabled(parsed.disabled);
        if (typeof parsed.autoPaste === 'boolean') setAutoPaste(parsed.autoPaste);
      }
    } catch { /* ignore */ }
    return () => {
      flog.info('overlay', 'unmounted');
      if (clickTimerRef.current) clearTimeout(clickTimerRef.current);
      if (collapseTimerRef.current) clearTimeout(collapseTimerRef.current);
      if (shrinkTimerRef.current) clearTimeout(shrinkTimerRef.current);
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

  // Subscribe to recording-cancelled for brief red X flash
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    listen('recording-cancelled', () => {
      if (timeoutId) clearTimeout(timeoutId);
      setShowCancelled(true);
      timeoutId = setTimeout(() => {
        if (!cancelled) setShowCancelled(false);
        timeoutId = null;
      }, 800);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => {
      cancelled = true;
      if (timeoutId) clearTimeout(timeoutId);
      unlisten?.();
    };
  }, []);

  // Subscribe to notch info changes (display config change: monitor plug/unplug, lid)
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<{ notch_width: number; notch_height: number } | null>('notch-info-changed', (event) => {
      // Rust resizes the window back to collapsed dimensions on display change,
      // so reset the expanded UI state to stay in sync.
      setExpanded(false);
      if (event.payload) {
        flog.info('overlay', 'notch info changed', { notch_width: event.payload.notch_width, notch_height: event.payload.notch_height });
        notchHeightRef.current = event.payload.notch_height;
        setNotchHeight(event.payload.notch_height);
        setNotchWidth(event.payload.notch_width);
      } else {
        flog.info('overlay', 'notch removed (no notch on new display)');
        notchHeightRef.current = 0;
        setNotchHeight(0);
        setNotchWidth(140);
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

  // Subscribe to app-disabled-changed events from Rust
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<boolean>('app-disabled-changed', (event) => {
      flog.info('overlay', 'app-disabled-changed', { disabled: event.payload });
      setDisabled(event.payload);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Subscribe to settings-changed (emitted by the main window) so the quick
  // controls reflect changes made there, even while already expanded.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen('settings-changed', () => {
      try {
        const s = loadSettings();
        setAutoPaste(s.autoPaste);
        setDisabled(s.disabled);
      } catch { /* ignore */ }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Track recording elapsed time for the inline timer (recording + hover only).
  useEffect(() => {
    if (status !== 'recording') { setElapsed(0); return; }
    const start = Date.now();
    setElapsed(0);
    const id = setInterval(() => setElapsed(Math.floor((Date.now() - start) / 1000)), 250);
    return () => clearInterval(id);
  }, [status]);

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
    if (disabledRef.current && currentStatus === 'idle') return;
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

  const isActive = status === 'recording' || status === 'processing' || showCancelled;

  // Raw mousedown — fires before click/double-click debouncing
  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    flog.info('overlay', 'MOUSEDOWN', {
      button: e.button, x: Math.round(e.clientX), y: Math.round(e.clientY),
      target: (e.target as HTMLElement).tagName,
      className: (e.target as HTMLElement).className?.slice(0, 50),
      locked: lockedRef.current, status: statusRef.current,
    });
  }, []);

  // Hover-expand: grow the window first, then animate the card open.
  const handleMouseEnter = useCallback(() => {
    if (collapseTimerRef.current) { clearTimeout(collapseTimerRef.current); collapseTimerRef.current = null; }
    if (shrinkTimerRef.current) { clearTimeout(shrinkTimerRef.current); shrinkTimerRef.current = null; }
    // Refresh quick-control values from localStorage (overlay has no shared settings context).
    try {
      const s = loadSettings();
      setAutoPaste(s.autoPaste);
      setDisabled(s.disabled);
    } catch { /* ignore */ }
    invoke('set_overlay_expanded', { expanded: true })
      .catch((e) => flog.warn('overlay', 'set_overlay_expanded(true) failed', { error: String(e) }));
    setExpanded(true);
  }, []);

  // Collapse after a 300ms hover-intent delay; shrink the window only after the
  // close animation finishes so the dropdown isn't clipped mid-transition.
  const handleMouseLeave = useCallback(() => {
    if (collapseTimerRef.current) clearTimeout(collapseTimerRef.current);
    collapseTimerRef.current = setTimeout(() => {
      collapseTimerRef.current = null;
      setExpanded(false);
      if (shrinkTimerRef.current) clearTimeout(shrinkTimerRef.current);
      shrinkTimerRef.current = setTimeout(() => {
        shrinkTimerRef.current = null;
        invoke('set_overlay_expanded', { expanded: false })
          .catch((e) => flog.warn('overlay', 'set_overlay_expanded(false) failed', { error: String(e) }));
      }, 380);
    }, 300);
  }, []);

  // Quick control: auto-paste. Write localStorage + notify the main window.
  const handleToggleAutoPaste = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      const s = loadSettings();
      const next = !s.autoPaste;
      saveSettings({ ...s, autoPaste: next });
      setAutoPaste(next);
      emit('settings-changed').catch((err) => flog.warn('overlay', 'emit settings-changed failed', { error: String(err) }));
    } catch (err) {
      flog.error('overlay', 'toggle autoPaste failed', { error: String(err) });
    }
  }, []);

  // Quick control: global disable. Gate the backend immediately, then notify.
  const handleToggleDisabled = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      const s = loadSettings();
      const next = !s.disabled;
      await invoke('set_app_disabled', { disabled: next });
      saveSettings({ ...s, disabled: next });
      setDisabled(next);
      emit('settings-changed').catch((err) => flog.warn('overlay', 'emit settings-changed failed', { error: String(err) }));
    } catch (err) {
      flog.error('overlay', 'toggle disabled failed', { error: String(err) });
    }
  }, []);

  // Quick control: open the main window's Settings panel.
  const handleOpenSettings = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await invoke('show_main_window');
      await emit('open-settings');
    } catch (err) {
      flog.error('overlay', 'open settings failed', { error: String(err) });
    }
  }, []);

  const topH = notchHeight || 37;

  return (
    <div
      className="w-full h-full flex"
      style={{ background: 'transparent' }}
      onMouseDown={handleMouseDown}
      onDoubleClick={handleDoubleClick}
      onClick={handleClick}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
    >
      {/* Dynamic Island: top bar matches notch height; hover expands it downward
          to reveal the quick-settings dropdown. Idle/recording only changes the
          top bar — the dropdown row is identical. */}
      <div
        className="cursor-pointer select-none overflow-hidden"
        style={{
          borderRadius: '0 0 12px 12px',
          width: (expanded || isActive) ? notchWidth + 68 : notchWidth + 28,
          height: expanded ? topH + 56 : topH,
          marginLeft: 32,
          background: 'rgba(20, 20, 20, 0.92)',
          backdropFilter: 'blur(40px)',
          WebkitBackdropFilter: 'blur(40px)',
          transition: 'width 400ms cubic-bezier(0.34,1.56,0.64,1), height 360ms cubic-bezier(0.34,1.56,0.64,1)',
        }}
      >
        {/* Top bar — the only draggable surface (keeps the dropdown buttons clickable) */}
        <div data-tauri-drag-region className="flex items-center" style={{ height: topH, paddingLeft: 10, paddingRight: 10 }}>
          {/* Left side — mic icon (idle) or red dot (recording) or spinner (processing) or red X (cancelled), all same position */}
          <div className="shrink-0 w-3 h-3 flex items-center justify-center">
            {showCancelled ? (
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#ef4444" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                <line x1="6" y1="6" x2="18" y2="18" />
                <line x1="18" y1="6" x2="6" y2="18" />
              </svg>
            ) : status === 'recording' ? (
              <div className="w-2.5 h-2.5 rounded-full bg-red-500" style={{ animation: 'pulse 0.8s ease-in-out infinite' }} />
            ) : status === 'processing' ? (
              <span className="w-3 h-3 border-[1.5px] border-white/20 border-t-white/70 rounded-full animate-spin block" />
            ) : (
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="rgba(255,255,255,0.4)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ opacity: disabled ? 0.15 : 1 }}>
                <rect x="9" y="1" width="6" height="12" rx="3" />
                <path d="M5 10a7 7 0 0 0 14 0" />
                <line x1="12" y1="17" x2="12" y2="21" />
              </svg>
            )}
          </div>

          {/* Inline timer — recording + hover only */}
          {status === 'recording' && expanded && (
            <span className="shrink-0 text-white/60 tabular-nums" style={{ marginLeft: 7, fontSize: 11 }}>
              {formatElapsed(elapsed)}
            </span>
          )}

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

        {/* Quick-settings dropdown — revealed on hover (identical in idle/recording) */}
        <div
          className="flex items-center justify-center gap-4"
          style={{
            height: 56,
            padding: '0 12px 8px',
            opacity: expanded ? 1 : 0,
            pointerEvents: expanded ? 'auto' : 'none',
            transition: 'opacity 200ms ease',
            transitionDelay: expanded ? '100ms' : '0ms',
          }}
        >
          {/* Global disable (speaker-slash) */}
          <button
            type="button"
            aria-label="Disable Murmur"
            onClick={handleToggleDisabled}
            className="shrink-0 flex items-center justify-center cursor-pointer rounded-[9px] transition-colors"
            style={{ width: 30, height: 30, background: disabled ? 'rgba(239,68,68,0.12)' : 'rgba(255,255,255,0.06)' }}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke={disabled ? '#ef4444' : 'rgba(255,255,255,0.85)'} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
              <line x1="23" y1="9" x2="17" y2="15" />
              <line x1="17" y1="9" x2="23" y2="15" />
            </svg>
          </button>

          {/* Auto-paste toggle */}
          <button
            type="button"
            role="switch"
            aria-checked={autoPaste}
            aria-label="Auto paste"
            onClick={handleToggleAutoPaste}
            className="relative shrink-0 cursor-pointer rounded-full transition-colors"
            style={{ width: 34, height: 18, opacity: disabled ? 0.35 : 1, background: autoPaste ? 'rgba(255,255,255,0.92)' : 'rgba(255,255,255,0.18)' }}
          >
            <span
              className="absolute rounded-full transition-transform"
              style={{ top: 2, left: 2, width: 14, height: 14, background: autoPaste ? '#14141a' : '#fff', transform: autoPaste ? 'translateX(16px)' : 'translateX(0)' }}
            />
          </button>

          {/* Open settings (gear) */}
          <button
            type="button"
            aria-label="Open settings"
            onClick={handleOpenSettings}
            className="shrink-0 flex items-center justify-center cursor-pointer rounded-[9px] transition-colors"
            style={{ width: 30, height: 30, background: 'rgba(255,255,255,0.06)' }}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="rgba(255,255,255,0.85)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <circle cx="12" cy="12" r="3" />
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
            </svg>
          </button>
        </div>
      </div>
    </div>
  );
}
