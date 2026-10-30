import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';

// T3's max-md breakpoint and 16rem / 3rem geometry at the app's 16px root.
export const HOME_SIDEBAR_QUERY = '(max-width: 767px)';
export const HOME_SIDEBAR_MIN = 256;
export const HOME_SIDEBAR_MAX = 384;
export const HOME_SIDEBAR_RAIL = 48;
const clamp = (width: number) => Math.max(HOME_SIDEBAR_MIN, Math.min(HOME_SIDEBAR_MAX, width));

interface Drag {
  pointerId: number;
  startX: number;
  startWidth: number;
  target: HTMLDivElement;
}

export function useHomeSidebar(initialCollapsed = false, suspended = false) {
  const [narrow, setNarrow] = useState(() => window.matchMedia?.(HOME_SIDEBAR_QUERY).matches ?? false);
  const [desktopCollapsed, setDesktopCollapsed] = useState(initialCollapsed);
  const [narrowExpanded, setNarrowExpanded] = useState(false);
  const [expandedWidth, setExpandedWidth] = useState(HOME_SIDEBAR_MIN);
  const [resizing, setResizing] = useState(false);
  const drag = useRef<Drag | null>(null);
  const collapsed = narrow ? !narrowExpanded : desktopCollapsed;

  const finish = useCallback((cancel: boolean) => {
    const current = drag.current;
    if (!current) return;
    drag.current = null;
    if (cancel) setExpandedWidth(current.startWidth);
    setResizing(false);
    if (current.target.hasPointerCapture(current.pointerId)) {
      current.target.releasePointerCapture(current.pointerId);
    }
  }, []);

  useEffect(() => {
    const media = window.matchMedia?.(HOME_SIDEBAR_QUERY);
    if (!media) return;
    const changed = () => {
      finish(true);
      setNarrow(media.matches);
      setNarrowExpanded(false);
    };
    media.addEventListener('change', changed);
    return () => media.removeEventListener('change', changed);
  }, [finish]);

  useEffect(() => {
    const cancel = () => finish(true);
    const hidden = () => { if (document.hidden) cancel(); };
    window.addEventListener('blur', cancel);
    document.addEventListener('visibilitychange', hidden);
    return () => {
      window.removeEventListener('blur', cancel);
      document.removeEventListener('visibilitychange', hidden);
      // Release ownership without setting state during unmount.
      const current = drag.current;
      drag.current = null;
      if (current?.target.hasPointerCapture(current.pointerId)) current.target.releasePointerCapture(current.pointerId);
    };
  }, [finish]);

  useEffect(() => { if (suspended) finish(true); }, [suspended, finish]);

  const toggle = () => {
    finish(true);
    if (narrow) setNarrowExpanded((value) => !value);
    else setDesktopCollapsed((value) => !value);
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (suspended || collapsed || drag.current || !event.isPrimary || event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.focus();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { pointerId: event.pointerId, startX: event.clientX, startWidth: expandedWidth, target: event.currentTarget };
    setResizing(true);
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointerId !== event.pointerId) return;
    // matchMedia change events can arrive after pointerup during a native resize.
    if ((window.matchMedia?.(HOME_SIDEBAR_QUERY).matches ?? narrow) !== narrow) {
      finish(true);
      return;
    }
    setExpandedWidth(clamp(current.startWidth + event.clientX - current.startX));
  };
  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current?.pointerId !== event.pointerId) return;
    onPointerMove(event);
    finish(false);
  };
  const onPointerCancel = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current?.pointerId === event.pointerId) finish(true);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape' && drag.current) {
      event.preventDefault();
      finish(true);
      return;
    }
    if (suspended || collapsed || drag.current) return;
    const step = event.shiftKey ? 32 : 16;
    const next = event.key === 'ArrowLeft' ? expandedWidth - step
      : event.key === 'ArrowRight' ? expandedWidth + step
      : event.key === 'Home' ? HOME_SIDEBAR_MIN
      : event.key === 'End' ? HOME_SIDEBAR_MAX : null;
    if (next === null) return;
    event.preventDefault();
    setExpandedWidth(clamp(next));
  };

  return {
    collapsed, resizing, toggle,
    width: collapsed ? HOME_SIDEBAR_RAIL : expandedWidth,
    resizeProps: { onPointerDown, onPointerMove, onPointerUp, onPointerCancel, onLostPointerCapture: onPointerCancel, onKeyDown },
  };
}
