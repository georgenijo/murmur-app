import { useState, useEffect, useCallback, useRef } from 'react';
import { listen, emit } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { isDictationStatus } from '../lib/types';
import type { DictationStatus } from '../lib/types';
import { flog } from '../lib/log';
import { STORAGE_KEY, DEFAULT_SETTINGS } from '../lib/settings';

const BAR_COUNT = 7;
const DROPDOWN_ROW_HEIGHT = 40;

function readSettingsFromStorage() {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) return JSON.parse(stored);
  } catch { /* ignore */ }
  return {};
}

function writeSettingToStorage(key: string, value: unknown) {
  try {
    const current = readSettingsFromStorage();
    const updated = { ...DEFAULT_SETTINGS, ...current, [key]: value };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(updated));
  } catch { /* ignore */ }
}

export function OverlayWidget() {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const [showCancelled, setShowCancelled] = useState(false);
  const [lockedMode, setLockedMode] = useState(false);
  const [notchWidth, setNotchWidth] = useState(185);
  const [notchHeight, setNotchHeight] = useState(37);
  const notchHeightRef = useRef(0);
  const statusRef = useRef(status);
  const lockedRef = useRef(lockedMode);
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const audioLevelRef = useRef(0);
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);

  // Hover state
  const [hovered, setHovered] = useState(false);
  const [showControls, setShowControls] = useState(false);
  const hoverTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const controlsFadeRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Quick settings state (read from localStorage)
  const [autoPaste, setAutoPaste] = useState(false);
  const [disabled, setDisabled] = useState(false);

  // Recording timer
  const [recordingStartTime, setRecordingStartTime] = useState<number | null>(null);
  const [recordingDuration, setRecordingDuration] = useState(0);

  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { lockedRef.current = lockedMode; }, [lockedMode]);

  // Track recording start time for timer
  useEffect(() => {
    if (status === 'recording') {
      setRecordingStartTime(Date.now());
      setRecordingDuration(0);
    } else {
      setRecordingStartTime(null);
      setRecordingDuration(0);
    }
  }, [status]);

  // Update recording timer every second while recording
  useEffect(() => {
    if (recordingStartTime === null) return;
    const interval = setInterval(() => {
      setRecordingDuration(Math.floor((Date.now() - recordingStartTime) / 1000));
    }, 1000);
    return () => clearInterval(interval);
  }, [recordingStartTime]);

  // Log mount + fetch notch dimensions + read settings
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

    // Read settings from localStorage
    const stored = readSettingsFromStorage();
    setAutoPaste(stored.autoPaste ?? DEFAULT_SETTINGS.autoPaste);
    setDisabled(stored.disabled ?? false);

    return () => {
      flog.info('overlay', 'unmounted');
      if (clickTimerRef.current) clearTimeout(clickTimerRef.current);
      if (hoverTimeoutRef.current) clearTimeout(hoverTimeoutRef.current);
      if (controlsFadeRef.current) clearTimeout(controlsFadeRef.current);
    };
  }, []);

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
      if (event.payload) {
        flog.info('overlay', 'notch info changed', { notch_width: event.payload.notch_width, notch_height: event.payload.notch_height });
        notchHeightRef.current = event.payload.notch_height;
        setNotchHeight(event.payload.notch_height);
        setNotchWidth(event.payload.notch_width);
      } else {
        flog.info('overlay', 'notch removed (no notch on new display)');
        notchHeightRef.current = 0;
        setNotchHeight(37);
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

  // Listen for settings-changed from main window (keeps overlay in sync)
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<Record<string, unknown>>('settings-changed', (event) => {
      const payload = event.payload;
      if ('autoPaste' in payload) setAutoPaste(payload.autoPaste as boolean);
      if ('disabled' in payload) setDisabled(payload.disabled as boolean);
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

  // Hover handlers
  const handleMouseEnter = useCallback(() => {
    if (hoverTimeoutRef.current) {
      clearTimeout(hoverTimeoutRef.current);
      hoverTimeoutRef.current = null;
    }
    setHovered(true);
    // Re-read settings from localStorage on each hover-expand
    const stored = readSettingsFromStorage();
    setAutoPaste(stored.autoPaste ?? DEFAULT_SETTINGS.autoPaste);
    setDisabled(stored.disabled ?? false);
    // Delayed controls fade-in
    controlsFadeRef.current = setTimeout(() => setShowControls(true), 100);
  }, []);

  const handleMouseLeave = useCallback(() => {
    if (controlsFadeRef.current) {
      clearTimeout(controlsFadeRef.current);
      controlsFadeRef.current = null;
    }
    // 300ms hover-intent delay before collapsing
    hoverTimeoutRef.current = setTimeout(() => {
      setHovered(false);
      setShowControls(false);
    }, 300);
  }, []);

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

  // Dropdown control handlers
  const handleToggleDisabled = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    const newVal = !disabled;
    setDisabled(newVal);
    writeSettingToStorage('disabled', newVal);
    emit('settings-changed', { disabled: newVal });
  }, [disabled]);

  const handleToggleAutoPaste = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    if (disabled) return;
    const newVal = !autoPaste;
    setAutoPaste(newVal);
    writeSettingToStorage('autoPaste', newVal);
    emit('settings-changed', { autoPaste: newVal });
  }, [autoPaste, disabled]);

  const handleOpenSettings = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    invoke('show_main_window').catch((err) =>
      flog.warn('overlay', 'show_main_window failed', { error: String(err) })
    );
  }, []);

  const pillHeight = hovered ? notchHeight + DROPDOWN_ROW_HEIGHT : notchHeight;

  const formatDuration = (seconds: number) => {
    const m = Math.floor(seconds / 60);
    const s = seconds % 60;
    return `${m}:${String(s).padStart(2, '0')}`;
  };

  return (
    <div
      data-tauri-drag-region
      className="w-full h-full flex"
      style={{ background: 'transparent' }}
      onMouseDown={handleMouseDown}
      onDoubleClick={handleDoubleClick}
      onClick={handleClick}
    >
      <div
        className="cursor-pointer select-none overflow-hidden"
        onMouseEnter={handleMouseEnter}
        onMouseLeave={handleMouseLeave}
        style={{
          borderRadius: '0 0 12px 12px',
          width: isActive ? notchWidth + 68 : notchWidth + 28,
          marginLeft: 32,
          height: pillHeight,
          background: 'rgba(20, 20, 20, 0.92)',
          backdropFilter: 'blur(40px)',
          WebkitBackdropFilter: 'blur(40px)',
          transition: 'width 500ms cubic-bezier(0.34, 1.56, 0.64, 1), height 350ms cubic-bezier(0.34, 1.56, 0.64, 1)',
        }}
      >
        {/* Top bar — same content as before, fixed height matching notch */}
        <div className="flex items-center shrink-0" style={{ height: notchHeight, paddingLeft: 10, paddingRight: 10 }}>
          {/* Left side — mic icon / red dot / spinner / red X */}
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
              <svg
                width="12" height="12" viewBox="0 0 24 24" fill="none"
                stroke={disabled ? 'rgba(255,255,255,0.15)' : 'rgba(255,255,255,0.4)'}
                strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
                style={{ transition: 'stroke 200ms ease' }}
              >
                <rect x="9" y="1" width="6" height="12" rx="3" />
                <path d="M5 10a7 7 0 0 0 14 0" />
                <line x1="12" y1="17" x2="12" y2="21" />
              </svg>
            )}
          </div>

          {/* Inline recording timer (visible only when recording + hovered) */}
          {status === 'recording' && hovered && (
            <span style={{
              fontSize: 10,
              color: 'rgba(255,255,255,0.6)',
              marginLeft: 4,
              fontVariantNumeric: 'tabular-nums',
              fontFamily: 'system-ui, -apple-system, sans-serif',
            }}>
              {formatDuration(recordingDuration)}
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

        {/* Dropdown controls row */}
        <div
          className="flex items-center justify-center gap-3"
          style={{
            height: DROPDOWN_ROW_HEIGHT,
            opacity: showControls ? 1 : 0,
            pointerEvents: showControls ? 'auto' : 'none',
            transition: 'opacity 200ms ease',
            paddingLeft: 12,
            paddingRight: 12,
          }}
        >
          {/* Speaker-slash icon — global disable toggle */}
          <button
            onClick={handleToggleDisabled}
            onDoubleClick={(e) => e.stopPropagation()}
            className="flex items-center justify-center rounded-full"
            style={{
              width: 28,
              height: 28,
              background: disabled ? 'rgba(239,68,68,0.12)' : 'transparent',
              transition: 'background 200ms ease',
              border: 'none',
              cursor: 'pointer',
              padding: 0,
            }}
          >
            <svg
              width="14" height="14" viewBox="0 0 24 24" fill="none"
              stroke={disabled ? '#ef4444' : 'rgba(255,255,255,0.5)'}
              strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
              style={{ transition: 'stroke 200ms ease' }}
            >
              <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
              <line x1="23" y1="9" x2="17" y2="15" />
              <line x1="17" y1="9" x2="23" y2="15" />
            </svg>
          </button>

          {/* Auto-paste toggle */}
          <button
            onClick={handleToggleAutoPaste}
            onDoubleClick={(e) => e.stopPropagation()}
            className="flex items-center justify-center rounded-full"
            style={{
              width: 28,
              height: 28,
              background: autoPaste && !disabled ? 'rgba(255,255,255,0.12)' : 'transparent',
              opacity: disabled ? 0.35 : 1,
              transition: 'background 200ms ease, opacity 200ms ease',
              border: 'none',
              cursor: disabled ? 'default' : 'pointer',
              padding: 0,
            }}
          >
            <svg
              width="14" height="14" viewBox="0 0 24 24" fill="none"
              stroke={autoPaste && !disabled ? 'rgba(255,255,255,0.9)' : 'rgba(255,255,255,0.5)'}
              strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
              style={{ transition: 'stroke 200ms ease' }}
            >
              <rect x="8" y="2" width="8" height="4" rx="1" ry="1" />
              <path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2" />
            </svg>
          </button>

          {/* Gear icon — open settings */}
          <button
            onClick={handleOpenSettings}
            onDoubleClick={(e) => e.stopPropagation()}
            className="flex items-center justify-center rounded-full"
            style={{
              width: 28,
              height: 28,
              background: 'transparent',
              border: 'none',
              cursor: 'pointer',
              padding: 0,
            }}
          >
            <svg
              width="14" height="14" viewBox="0 0 24 24" fill="none"
              stroke="rgba(255,255,255,0.5)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
            >
              <circle cx="12" cy="12" r="3" />
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
            </svg>
          </button>
        </div>
      </div>
    </div>
  );
}
