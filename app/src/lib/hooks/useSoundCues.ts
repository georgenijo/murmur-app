import { useEffect, useRef } from 'react';
import type { MeetingRuntimePhase } from '../meetings';
import { playSoundCue } from '../soundCues';

interface SoundCueSettings {
  soundCuesEnabled: boolean;
  soundCueVolume: number;
  meetingSoundCuesEnabled: boolean;
}

export function useSoundCues(settings: SoundCueSettings, meetingPhase: MeetingRuntimePhase): void {
  const settingsRef = useRef(settings);
  const previousMeetingPhaseRef = useRef(meetingPhase);
  useEffect(() => { settingsRef.current = settings; }, [settings]);

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
