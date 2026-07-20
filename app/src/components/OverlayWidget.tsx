import { useState, useEffect, useCallback, useRef } from 'react';
import { listen, emit } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { cursorPosition, getCurrentWindow } from '@tauri-apps/api/window';
import { isDictationStatus } from '../lib/types';
import type { DictationStatus } from '../lib/types';
import { flog } from '../lib/log';
import { STORAGE_KEY, DEFAULT_SETTINGS, loadSettings, saveSettings } from '../lib/settings';
import type { Settings } from '../lib/settings';
import { buildConfigureOptions } from '../lib/dictation';
import type { DictationResponse } from '../lib/dictation';
import {
  HOTKEY_MISS_FLASH_MS,
  isHotkeyTapRejectedPayload,
  shouldShowHotkeyMissFeedback,
} from '../lib/hotkeyFeedback';

const BAR_COUNT = 7;
const COLLAPSE_DELAY_MS = 300;
const SHRINK_DELAY_MS = 380;
const HOVER_WATCHDOG_MS = 150;
const HOVER_BOUNDS_PADDING = 8;
const HOVER_OPEN_DWELL_MS = 150;
const DROPDOWN_H = 44;
const OVERLAY_HORIZONTAL_EXPANSION = 120;
const FALLBACK_NOTCH_WIDTH = 80;

function formatElapsed(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${String(s).padStart(2, '0')}`;
}

function PowerIcon({ stroke }: { stroke: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={stroke} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 2v10" />
      <path d="M18.4 6.6a9 9 0 1 1-12.8 0" />
    </svg>
  );
}

function ClipboardPasteIcon({ stroke }: { stroke: string }) {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke={stroke} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="8" y="2" width="8" height="4" rx="1" />
      <path d="M16 4h2a2 2 0 0 1 2 2v4" />
      <path d="M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h5" />
      <path d="M16 14v6" />
      <path d="M13 17h6" />
    </svg>
  );
}

function SlidersIcon({ stroke }: { stroke: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={stroke} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 7h4" />
      <path d="M12 7h8" />
      <circle cx="10" cy="7" r="2" />
      <path d="M4 17h10" />
      <path d="M18 17h2" />
      <circle cx="16" cy="17" r="2" />
    </svg>
  );
}

export function OverlayWidget() {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const [showCancelled, setShowCancelled] = useState(false);
  const [showHotkeyMiss, setShowHotkeyMiss] = useState(false);
  const [lockedMode, setLockedMode] = useState(false);
  const [disabled, setDisabled] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [autoPaste, setAutoPaste] = useState(false);
  const [fileOutputEnabled, setFileOutputEnabled] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const notchHeightRef = useRef(0);
  const [notchHeight, setNotchHeight] = useState(0);
  const [notchWidth, setNotchWidth] = useState(185);
  const islandRef = useRef<HTMLDivElement | null>(null);
  const lockedRef = useRef(lockedMode);
  const disabledRef = useRef(disabled);
  const expandedRef = useRef(expanded);
  const clickTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const collapseTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const shrinkTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dwellTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hotkeyMissTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const audioLevelRef = useRef(0);
  const hotkeyMissFeedbackRef = useRef(false);
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);
  const statusRef = useRef(status);

  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { lockedRef.current = lockedMode; }, [lockedMode]);
  useEffect(() => { disabledRef.current = disabled; }, [disabled]);
  useEffect(() => { expandedRef.current = expanded; }, [expanded]);

  const applySettingsSnapshot = useCallback((settings: Settings) => {
    setDisabled(settings.disabled);
    setAutoPaste(settings.autoPaste);
    setFileOutputEnabled(settings.saveTranscript || settings.saveAudio);
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
          setNotchWidth(FALLBACK_NOTCH_WIDTH);
        }
      })
      .catch((e) => flog.warn('overlay', 'get_notch_info failed', { error: String(e) }));
    try {
      applySettingsSnapshot(loadSettings());
    } catch { /* ignore */ }
    return () => {
      flog.info('overlay', 'unmounted');
      if (clickTimerRef.current) clearTimeout(clickTimerRef.current);
      if (collapseTimerRef.current) clearTimeout(collapseTimerRef.current);
      if (shrinkTimerRef.current) clearTimeout(shrinkTimerRef.current);
      if (dwellTimerRef.current) clearTimeout(dwellTimerRef.current);
      if (hotkeyMissTimerRef.current) clearTimeout(hotkeyMissTimerRef.current);
    };
  }, [applySettingsSnapshot]);

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

  // Subscribe to recording status events from Rust.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<unknown>('recording-status-changed', (event) => {
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
        setNotchWidth(FALLBACK_NOTCH_WIDTH);
      }
      invoke('set_overlay_expanded', { expanded: false })
        .catch((e) => flog.warn('overlay', 'display-change surface sync failed', { error: String(e) }));
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
        applySettingsSnapshot(s);
      } catch { /* ignore */ }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [applySettingsSnapshot]);

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
    if (expandedRef.current) {
      flog.info('overlay', 'double-click ignored while expanded', { status: currentStatus });
      return;
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

  const shrinkOverlayWindow = useCallback(() => {
    if (shrinkTimerRef.current) clearTimeout(shrinkTimerRef.current);
    shrinkTimerRef.current = setTimeout(() => {
      shrinkTimerRef.current = null;
      invoke('set_overlay_expanded', { expanded: false })
        .catch((e) => flog.warn('overlay', 'set_overlay_expanded(false) failed', { error: String(e) }));
    }, SHRINK_DELAY_MS);
  }, []);

  const collapseOverlay = useCallback((delayMs = COLLAPSE_DELAY_MS) => {
    if (collapseTimerRef.current) clearTimeout(collapseTimerRef.current);
    collapseTimerRef.current = setTimeout(() => {
      collapseTimerRef.current = null;
      setExpanded(false);
      shrinkOverlayWindow();
    }, delayMs);
  }, [shrinkOverlayWindow]);

  // Hover-expand: grow the window first, then animate the card open.
  const openOverlay = useCallback(() => {
    if (dwellTimerRef.current) { clearTimeout(dwellTimerRef.current); dwellTimerRef.current = null; }
    if (expandedRef.current) return;
    // Refresh quick-control values from localStorage (overlay has no shared settings context).
    try {
      const s = loadSettings();
      applySettingsSnapshot(s);
    } catch { /* ignore */ }
    invoke('set_overlay_expanded', { expanded: true })
      .catch((e) => flog.warn('overlay', 'set_overlay_expanded(true) failed', { error: String(e) }));
    setExpanded(true);
  }, [applySettingsSnapshot]);

  // Opening requires hover intent: the cursor must dwell on the island before
  // the card expands, so grazing the notch no longer pops the dropdown.
  const noteHoverStart = useCallback(() => {
    if (collapseTimerRef.current) { clearTimeout(collapseTimerRef.current); collapseTimerRef.current = null; }
    if (shrinkTimerRef.current) { clearTimeout(shrinkTimerRef.current); shrinkTimerRef.current = null; }
    if (expandedRef.current || dwellTimerRef.current) return;
    dwellTimerRef.current = setTimeout(() => {
      dwellTimerRef.current = null;
      openOverlay();
    }, HOVER_OPEN_DWELL_MS);
  }, [openOverlay]);

  const cancelHoverDwell = useCallback(() => {
    if (dwellTimerRef.current) { clearTimeout(dwellTimerRef.current); dwellTimerRef.current = null; }
  }, []);

  // Collapse after a 300ms hover-intent delay; shrink the window only after the
  // close animation finishes so the dropdown isn't clipped mid-transition.
  const handleMouseLeave = useCallback(() => {
    cancelHoverDwell();
    collapseOverlay();
  }, [cancelHoverDwell, collapseOverlay]);

  // Safety net for macOS/Tauri hover edge cases: if the native window is already
  // expanded but a leave event is missed, collapse once the cursor is outside the
  // actual visible island card (not merely outside the transparent window frame).
  useEffect(() => {
    if (!expanded) return;
    const currentWindow = getCurrentWindow();
    const intervalId = setInterval(async () => {
      const island = islandRef.current;
      if (!island || !expandedRef.current) return;
      try {
        const [windowPosition, cursor] = await Promise.all([
          currentWindow.outerPosition(),
          cursorPosition(),
        ]);
        const scale = window.devicePixelRatio || 1;
        const rect = island.getBoundingClientRect();
        const padding = HOVER_BOUNDS_PADDING * scale;
        const left = windowPosition.x + rect.left * scale - padding;
        const right = windowPosition.x + rect.right * scale + padding;
        const top = windowPosition.y + rect.top * scale - padding;
        const bottom = windowPosition.y + rect.bottom * scale + padding;

        if (cursor.x < left || cursor.x > right || cursor.y < top || cursor.y > bottom) {
          collapseOverlay(0);
        }
      } catch (err) {
        flog.warn('overlay', 'hover watchdog failed', { error: String(err) });
      }
    }, HOVER_WATCHDOG_MS);
    return () => clearInterval(intervalId);
  }, [expanded, collapseOverlay]);

  // The overlay is non-activating and sits above the menu bar, so macOS can miss
  // normal DOM hover entry events. Polling the cursor against the visible island
  // bounds keeps hover-expand reliable without widening the clickable window.
  // Entry bounds are strict (no padding) and only arm the dwell timer — the
  // card opens after HOVER_OPEN_DWELL_MS of sustained hover, not on a graze.
  useEffect(() => {
    const currentWindow = getCurrentWindow();
    let inFlight = false;
    const intervalId = setInterval(async () => {
      const island = islandRef.current;
      if (!island || expandedRef.current || inFlight) return;
      inFlight = true;
      try {
        const [windowPosition, cursor] = await Promise.all([
          currentWindow.outerPosition(),
          cursorPosition(),
        ]);
        const scale = window.devicePixelRatio || 1;
        const rect = island.getBoundingClientRect();
        const left = windowPosition.x + rect.left * scale;
        const right = windowPosition.x + rect.right * scale;
        const top = windowPosition.y + rect.top * scale;
        const bottom = windowPosition.y + rect.bottom * scale;

        if (cursor.x >= left && cursor.x <= right && cursor.y >= top && cursor.y <= bottom) {
          noteHoverStart();
        } else {
          cancelHoverDwell();
        }
      } catch (err) {
        flog.warn('overlay', 'hover detector failed', { error: String(err) });
      } finally {
        inFlight = false;
      }
    }, HOVER_WATCHDOG_MS);
    return () => clearInterval(intervalId);
  }, [noteHoverStart, cancelHoverDwell]);

  // Quick control: auto-paste. Write localStorage + notify the main window.
  const handleToggleAutoPaste = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      const s = loadSettings();
      const next = !s.autoPaste;
      const nextSettings = { ...s, autoPaste: next };
      saveSettings(nextSettings);
      applySettingsSnapshot(nextSettings);
      try {
        await invoke('configure_dictation', { options: buildConfigureOptions(nextSettings) });
      } catch (err) {
        saveSettings(s);
        applySettingsSnapshot(s);
        throw err;
      }
      emit('settings-changed').catch((err) => flog.warn('overlay', 'emit settings-changed failed', { error: String(err) }));
    } catch (err) {
      flog.error('overlay', 'toggle autoPaste failed', { error: String(err) });
      try {
        applySettingsSnapshot(loadSettings());
      } catch { /* ignore */ }
    }
  }, [applySettingsSnapshot]);

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
  const effectiveAutoPaste = autoPaste && !fileOutputEnabled;
  const autoPastePaused = autoPaste && fileOutputEnabled;
  const autoPasteLabel = autoPastePaused
    ? 'Auto-paste paused while saving files'
    : effectiveAutoPaste
      ? 'Disable auto-paste'
      : 'Enable auto-paste';
  const autoPasteColor = effectiveAutoPaste
    ? '#10b981'
    : autoPastePaused
      ? '#f59e0b'
      : 'rgba(255,255,255,0.85)';
  const autoPasteBackground = effectiveAutoPaste
    ? 'rgba(16,185,129,0.16)'
    : autoPastePaused
      ? 'rgba(245,158,11,0.14)'
      : 'rgba(255,255,255,0.06)';

  return (
    <div
      className="w-full h-full flex"
      style={{ background: 'transparent' }}
      onMouseDown={handleMouseDown}
      onDoubleClick={handleDoubleClick}
      onClick={handleClick}
      onMouseEnter={noteHoverStart}
      onMouseMove={noteHoverStart}
    >
      {/* Dynamic Island: top bar matches notch height; hover expands it downward
          to reveal the quick-settings dropdown. Idle/recording only changes the
          top bar — the dropdown row is identical. */}
      <div
        ref={islandRef}
        className="cursor-pointer select-none overflow-hidden"
        onMouseEnter={noteHoverStart}
        onMouseMove={noteHoverStart}
        onMouseLeave={handleMouseLeave}
        style={{
          borderRadius: '0 0 12px 12px',
          width: (expanded || isActive)
            ? notchWidth + OVERLAY_HORIZONTAL_EXPANSION
            : notchWidth + 28,
          height: topH + (expanded ? DROPDOWN_H : 0),
          marginLeft: (expanded || isActive)
            ? 0
            : (OVERLAY_HORIZONTAL_EXPANSION - 28) / 2,
          background: 'rgba(20, 20, 20, 0.92)',
          boxShadow: showHotkeyMiss ? 'inset 0 -2px 0 rgba(245,158,11,0.9), 0 3px 16px rgba(245,158,11,0.22)' : 'none',
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

          {/* Recording time remains in the visible left wing, outside the physical notch. */}
          {status === 'recording' && (
            <span className="shrink-0 text-white/60 tabular-nums" style={{ marginLeft: 7, fontSize: 11 }}>
              {formatElapsed(elapsed)}
            </span>
          )}

          {/* This spacer is intentionally the notch-obscured center region. */}
          <div className="flex-1" aria-hidden="true" />

          {/* Right side — waveform (only when active) */}
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

        {/* Quick-settings dropdown — revealed on hover (identical in idle/recording) */}
        <div
          className="flex items-center justify-center gap-3"
          style={{
            height: DROPDOWN_H,
            padding: '0 10px 6px',
            opacity: expanded ? 1 : 0,
            pointerEvents: expanded ? 'auto' : 'none',
            transition: 'opacity 200ms ease',
            transitionDelay: expanded ? '100ms' : '0ms',
          }}
        >
          {/* Global disable */}
          <button
            type="button"
            aria-label={disabled ? 'Enable Murmur' : 'Disable Murmur'}
            onClick={handleToggleDisabled}
            className="shrink-0 flex items-center justify-center cursor-pointer rounded-[9px] transition-colors"
            style={{ width: 26, height: 26, background: disabled ? 'rgba(239,68,68,0.12)' : 'rgba(255,255,255,0.06)' }}
          >
            <PowerIcon stroke={disabled ? '#ef4444' : 'rgba(255,255,255,0.85)'} />
          </button>

          {/* Auto-paste */}
          <button
            type="button"
            role="switch"
            aria-checked={effectiveAutoPaste}
            aria-label={autoPasteLabel}
            title={autoPasteLabel}
            onClick={handleToggleAutoPaste}
            className="shrink-0 flex items-center justify-center cursor-pointer rounded-[9px] transition-colors"
            style={{ width: 26, height: 26, opacity: disabled ? 0.35 : 1, background: autoPasteBackground }}
          >
            <ClipboardPasteIcon stroke={autoPasteColor} />
          </button>

          {/* Open settings */}
          <button
            type="button"
            aria-label="Open settings"
            onClick={handleOpenSettings}
            className="shrink-0 flex items-center justify-center cursor-pointer rounded-[9px] transition-colors"
            style={{ width: 26, height: 26, background: 'rgba(255,255,255,0.06)' }}
          >
            <SlidersIcon stroke="rgba(255,255,255,0.85)" />
          </button>
        </div>
      </div>
    </div>
  );
}
