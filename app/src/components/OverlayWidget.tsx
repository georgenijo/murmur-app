import { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { isDictationStatus } from '../lib/types';
import type { DictationStatus } from '../lib/types';
import { flog } from '../lib/log';
import { STORAGE_KEY, DEFAULT_SETTINGS, loadSettings } from '../lib/settings';
import type { Settings } from '../lib/settings';
import type { DictationResponse } from '../lib/dictation';
import {
  HOTKEY_MISS_FLASH_MS,
  isHotkeyTapRejectedPayload,
  shouldShowHotkeyMissFeedback,
} from '../lib/hotkeyFeedback';

const BAR_COUNT = 7;

export function OverlayWidget() {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const [showCancelled, setShowCancelled] = useState(false);
  const [showHotkeyMiss, setShowHotkeyMiss] = useState(false);
  const [lockedMode, setLockedMode] = useState(false);
  const [disabled, setDisabled] = useState(false);
  const notchHeightRef = useRef(0);
  const [notchHeight, setNotchHeight] = useState(0);
  const [notchWidth, setNotchWidth] = useState(185);
  const statusRef = useRef(status);
  const lockedRef = useRef(lockedMode);
  const disabledRef = useRef(disabled);
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hotkeyMissTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const audioLevelRef = useRef(0);
  const hotkeyMissFeedbackRef = useRef(false);
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);

  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { lockedRef.current = lockedMode; }, [lockedMode]);
  useEffect(() => { disabledRef.current = disabled; }, [disabled]);

  const applySettingsSnapshot = useCallback((settings: Settings) => {
    setDisabled(settings.disabled);
    hotkeyMissFeedbackRef.current = settings.hotkeyMissFeedback;
    if (!settings.hotkeyMissFeedback) setShowHotkeyMiss(false);
  }, []);

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
      applySettingsSnapshot(loadSettings());
    } catch { /* ignore */ }
    return () => {
      flog.info('overlay', 'unmounted');
      if (clickTimerRef.current) clearTimeout(clickTimerRef.current);
      if (hotkeyMissTimerRef.current) clearTimeout(hotkeyMissTimerRef.current);
    };
  }, [applySettingsSnapshot]);

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
        } else {
          setShowHotkeyMiss(false);
          if (hotkeyMissTimerRef.current) {
            clearTimeout(hotkeyMissTimerRef.current);
            hotkeyMissTimerRef.current = null;
          }
        }
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // A rejected-tap event is emitted only when the double-tap window expires.
  // The setting gate lives here because the overlay is a separate webview with
  // its own React context and reads the shared localStorage settings snapshot.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<unknown>('hotkey-tap-rejected', (event) => {
      if (!isHotkeyTapRejectedPayload(event.payload)) return;
      let feedbackEnabled = hotkeyMissFeedbackRef.current;
      try {
        feedbackEnabled = loadSettings().hotkeyMissFeedback;
        hotkeyMissFeedbackRef.current = feedbackEnabled;
      } catch { /* use the latest settings snapshot */ }
      if (!shouldShowHotkeyMissFeedback(
        feedbackEnabled,
        statusRef.current,
        event.payload,
      )) return;

      if (hotkeyMissTimerRef.current) clearTimeout(hotkeyMissTimerRef.current);
      setShowHotkeyMiss(true);
      hotkeyMissTimerRef.current = setTimeout(() => {
        if (!cancelled) setShowHotkeyMiss(false);
        hotkeyMissTimerRef.current = null;
      }, HOTKEY_MISS_FLASH_MS);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => {
      cancelled = true;
      if (hotkeyMissTimerRef.current) clearTimeout(hotkeyMissTimerRef.current);
      unlisten?.();
    };
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

  // Subscribe to settings-changed (emitted by the main window) so the disabled
  // dimming and hotkey-miss gate reflect changes made there.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen('settings-changed', () => {
      try {
        applySettingsSnapshot(loadSettings());
      } catch { /* ignore */ }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [applySettingsSnapshot]);

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
          const res = await invoke<DictationResponse>('start_native_recording', { deviceName });
          flog.info('overlay', 'start_native_recording result', { type: res.type, state: res.state });
          if (res.type !== 'recording_started') {
            flog.warn('overlay', 'recording start declined', { type: res.type });
            setLockedMode(false);
          }
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

  const isActive = status === 'recording' || status === 'processing' || showCancelled || showHotkeyMiss;

  // Raw mousedown — fires before click/double-click debouncing
  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    flog.info('overlay', 'MOUSEDOWN', {
      button: e.button, x: Math.round(e.clientX), y: Math.round(e.clientY),
      target: (e.target as HTMLElement).tagName,
      className: (e.target as HTMLElement).className?.slice(0, 50),
      locked: lockedRef.current, status: statusRef.current,
    });
  }, []);

  const topH = notchHeight || 37;

  return (
    <div
      data-tauri-drag-region
      className="w-full h-full flex"
      style={{ background: 'transparent' }}
      onMouseDown={handleMouseDown}
      onDoubleClick={handleDoubleClick}
      onClick={handleClick}
    >
      {/* Dynamic Island: same height as the notch. Idle = flush with the notch
          plus a small left tab holding the mic icon. Active = grows to the
          right to reveal the waveform. */}
      <div
        className="cursor-pointer select-none overflow-hidden"
        style={{
          borderRadius: '0 0 12px 12px',
          width: isActive ? notchWidth + 68 : notchWidth + 28,
          height: topH,
          marginLeft: 32,
          background: 'rgba(20, 20, 20, 0.92)',
          boxShadow: showHotkeyMiss ? 'inset 0 -2px 0 rgba(245,158,11,0.9), 0 3px 16px rgba(245,158,11,0.22)' : 'none',
          backdropFilter: 'blur(40px)',
          WebkitBackdropFilter: 'blur(40px)',
          transition: 'width 400ms cubic-bezier(0.34,1.56,0.64,1)',
        }}
      >
        <div className="flex items-center h-full" style={{ paddingLeft: 10, paddingRight: 10 }}>
          {/* Left side — mic icon (idle) or red dot (recording) or spinner (processing) or red X (cancelled), all same position */}
          <div className="shrink-0 w-3 h-3 flex items-center justify-center">
            {showCancelled ? (
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#ef4444" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                <line x1="6" y1="6" x2="18" y2="18" />
                <line x1="18" y1="6" x2="6" y2="18" />
              </svg>
            ) : showHotkeyMiss ? (
              <span className="w-3 h-3 rounded-full border border-amber-400 text-amber-300 text-[8px] leading-none flex items-center justify-center font-bold">
                !
              </span>
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

          {/* Spacer */}
          <div className="flex-1" />

          {/* Right side — waveform (recording) or hotkey-miss text */}
          {showHotkeyMiss ? (
            <span className="shrink-0 text-amber-300 text-[10px] font-medium">
              Tap missed
            </span>
          ) : (
            <div
              className="flex items-center gap-[1.5px] h-4 shrink-0 transition-opacity duration-300"
              style={{ opacity: status === 'recording' ? 1 : 0 }}
            >
              {Array.from({ length: BAR_COUNT }, (_, i) => (
                <div
                  key={i}
                  ref={el => { barRefs.current[i] = el; }}
                  className="w-[2px] rounded-full bg-white/90"
                  style={{
                    height: '2px',
                    transition: `height ${status === 'recording' ? '50ms' : '300ms'} ease-out`,
                  }}
                />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
