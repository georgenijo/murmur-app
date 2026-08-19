import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { MeetingRuntimePhase } from '../meetings';
import { playSoundCue, type SoundCue } from '../soundCues';

interface SoundCueSettings {
  soundCuesEnabled: boolean;
  soundCueVolume: number;
  meetingSoundCuesEnabled: boolean;
}

function deliveryCue(payload: unknown): SoundCue | null {
  const outcome = (payload as { outcome?: unknown } | null)?.outcome;
  return outcome === 'clipboard_only' || outcome === 'auto_pasted'
    ? 'success'
    : outcome === 'unconfirmed' ? 'failure' : null;
}

export function useSoundCues(settings: SoundCueSettings, meetingPhase: MeetingRuntimePhase): void {
  const settingsRef = useRef(settings);
  const meetingPhaseRef = useRef(meetingPhase);
  const previousMeetingPhaseRef = useRef(meetingPhase);
  useEffect(() => { settingsRef.current = settings; }, [settings]);
  useEffect(() => { meetingPhaseRef.current = meetingPhase; }, [meetingPhase]);

  useEffect(() => {
    const play = (cue: SoundCue) => {
      const current = settingsRef.current;
      if (!current.soundCuesEnabled || !['idle', 'failed'].includes(meetingPhaseRef.current)) return;
      playSoundCue(cue, current.soundCueVolume);
    };
    let cancelled = false;
    const unlistens: Array<() => void> = [];
    const subscribe = (event: string, cue: SoundCue | ((payload: unknown) => SoundCue | null)) => {
      listen<unknown>(event, ({ payload }) => {
        const resolved = typeof cue === 'function' ? cue(payload) : cue;
        if (resolved) play(resolved);
      }).then((unlisten) => cancelled ? unlisten() : unlistens.push(unlisten)).catch(() => {});
    };
    subscribe('dictation-generation-started', 'start');
    subscribe('recording-status-changed', (payload) => payload === 'processing' ? 'stop' : null);
    subscribe('dictation-delivery-outcome', deliveryCue);
    subscribe('recording-initialization-failed', 'failure');
    subscribe('recording-interrupted', 'failure');
    return () => {
      cancelled = true;
      unlistens.forEach((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    const previous = previousMeetingPhaseRef.current;
    previousMeetingPhaseRef.current = meetingPhase;
    const current = settingsRef.current;
    if (!current.soundCuesEnabled || !current.meetingSoundCuesEnabled || previous === meetingPhase) return;
    if (previous === 'idle' && meetingPhase === 'starting') playSoundCue('start', current.soundCueVolume);
    else if (meetingPhase === 'stopping') playSoundCue('stop', current.soundCueVolume);
    else if (meetingPhase === 'idle' && previous === 'processing') playSoundCue('success', current.soundCueVolume);
    else if (meetingPhase === 'failed') playSoundCue('failure', current.soundCueVolume);
  }, [meetingPhase]);
}
