import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { MeetingRuntimePhase } from '../meetings';
import { reportStartCueFinished } from '../dictation';
import { playSoundCue, type SoundCue } from '../soundCues';

interface SoundCueSettings {
  soundCuesEnabled: boolean;
  soundCueVolume: number;
  meetingSoundCuesEnabled: boolean;
  /** When on, report each start cue's play-out so the speakers mute after it. */
  muteSpeakersWhileDictating?: boolean;
}

function deliveryCue(payload: unknown): SoundCue | null {
  const outcome = (payload as { outcome?: unknown } | null)?.outcome;
  return outcome === 'autoPastePosted' || outcome === 'clipboardOnly'
    ? 'success'
    : outcome === 'unconfirmed' || outcome === 'clipboardWriteFailed' ? 'failure' : null;
}

export function useSoundCues(settings: SoundCueSettings, meetingPhase: MeetingRuntimePhase): void {
  const settingsRef = useRef(settings);
  const meetingPhaseRef = useRef(meetingPhase);
  const previousMeetingPhaseRef = useRef(meetingPhase);
  useEffect(() => { settingsRef.current = settings; }, [settings]);
  useEffect(() => { meetingPhaseRef.current = meetingPhase; }, [meetingPhase]);

  useEffect(() => {
    const play = (cue: SoundCue): Promise<void> | void => {
      const current = settingsRef.current;
      if (!current.soundCuesEnabled || !['idle', 'failed'].includes(meetingPhaseRef.current)) return;
      return playSoundCue(cue, current.soundCueVolume);
    };
    // The speakers mute only once the start cue has played out, or at once
    // when there is no cue to wait for.
    const startCue = (payload: unknown) => {
      const played = play('start');
      const recordingId = (payload as { recordingId?: unknown } | null)?.recordingId;
      if (!settingsRef.current.muteSpeakersWhileDictating || typeof recordingId !== 'number') return;
      void Promise.resolve(played).then(() => reportStartCueFinished(recordingId));
    };
    let cancelled = false;
    const unlistens: Array<() => void> = [];
    const register = (event: string, handler: (payload: unknown) => void) => {
      listen<unknown>(event, ({ payload }) => handler(payload))
        .then((unlisten) => cancelled ? unlisten() : unlistens.push(unlisten)).catch(() => {});
    };
    const subscribe = (event: string, cue: SoundCue | ((payload: unknown) => SoundCue | null)) => {
      register(event, (payload) => {
        const resolved = typeof cue === 'function' ? cue(payload) : cue;
        if (resolved) play(resolved);
      });
    };
    register('dictation-generation-started', startCue);
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
