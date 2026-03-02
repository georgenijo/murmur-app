import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AppEvent, StreamName, LevelName } from '../events';

const MAX_EVENTS = 500;

export function useEventStore() {
  const bufRef = useRef<AppEvent[]>([]);
  const [events, setEvents] = useState<AppEvent[]>([]);
  const rafRef = useRef<number | null>(null);

  const scheduleUpdate = useCallback(() => {
    if (rafRef.current !== null) return;
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      setEvents([...bufRef.current]);
    });
  }, []);

  useEffect(() => {
    let cancelled = false;

    // Hydrate from backend
    invoke<AppEvent[]>('get_event_history')
      .then((history) => {
        if (cancelled) return;
        bufRef.current = history.slice(-MAX_EVENTS);
        setEvents([...bufRef.current]);
      })
      .catch(() => {});

    // Listen for new events
    let unlisten: (() => void) | undefined;
    listen<AppEvent>('app-event', (e) => {
      if (cancelled) return;
      const buf = bufRef.current;
      buf.push(e.payload);
      if (buf.length > MAX_EVENTS) {
        buf.splice(0, buf.length - MAX_EVENTS);
      }
      scheduleUpdate();
    }).then((fn) => {
      if (cancelled) { fn(); return; }
      unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
      }
    };
  }, [scheduleUpdate]);

  const getByStream = useCallback((stream: StreamName) => {
    return bufRef.current.filter(e => e.stream === stream);
  }, []);

  const getByLevel = useCallback((level: LevelName) => {
    return bufRef.current.filter(e => e.level === level);
  }, []);

  const clear = useCallback(() => {
    invoke('clear_event_history').catch(() => {});
    bufRef.current = [];
    setEvents([]);
  }, []);

  return { events, getByStream, getByLevel, clear };
}
