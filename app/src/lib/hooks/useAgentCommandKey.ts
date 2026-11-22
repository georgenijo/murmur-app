import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { setAgentCommandShortcut } from '../dictation';
import type { AgentCommandKey } from '../settings';

interface UseAgentCommandKeyProps {
  /** The setting is on, the app is ready and Accessibility is granted. */
  enabled: boolean;
  shortcut: AgentCommandKey;
  onStart: () => void;
  onStop: () => void;
  /** The hold turned out to be a shortcut: discard the recording. */
  onCancel: () => void;
}

/**
 * Binds the hold-to-talk agent command key while `enabled`, and forwards the
 * native hold events. The key is unbound again whenever the setting turns off,
 * so nothing can start an agent command without the user's opt-in.
 */
export function useAgentCommandKey({ enabled, shortcut, onStart, onStop, onCancel }: UseAgentCommandKeyProps) {
  const onStartRef = useRef(onStart);
  const onStopRef = useRef(onStop);
  const onCancelRef = useRef(onCancel);
  useEffect(() => { onStartRef.current = onStart; }, [onStart]);
  useEffect(() => { onStopRef.current = onStop; }, [onStop]);
  useEffect(() => { onCancelRef.current = onCancel; }, [onCancel]);

  useEffect(() => {
    if (!enabled) return;

    let cancelled = false;
    const unlistens: (() => void)[] = [];
    const subscribe = async (event: string, handler: () => void) => {
      const unlisten = await listen(event, () => { if (!cancelled) handler(); });
      if (cancelled) unlisten(); else unlistens.push(unlisten);
    };

    const setup = async () => {
      // Subscribe before binding so the first hold cannot be missed.
      await subscribe('agent-command-start', () => onStartRef.current());
      await subscribe('agent-command-stop', () => onStopRef.current());
      await subscribe('agent-command-cancel', () => onCancelRef.current());
      if (cancelled) return;
      try {
        await setAgentCommandShortcut(shortcut);
        if (cancelled) setAgentCommandShortcut(null).catch(() => {});
      } catch (err) {
        console.error('Failed to bind the agent command key:', err);
      }
    };
    void setup();

    return () => {
      cancelled = true;
      unlistens.forEach((unlisten) => unlisten());
      // Nothing listens for the release any more, so a hold in progress is
      // discarded here rather than left recording. A no-op without one.
      onCancelRef.current();
      setAgentCommandShortcut(null).catch(() => {});
    };
  }, [enabled, shortcut]);
}
