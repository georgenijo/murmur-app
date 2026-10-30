import type { Settings } from '../../lib/settings';

/** Explicit owner routes: search reveals UI state, never backend preferences. */
export const ADVANCED_TARGETS: Record<string, readonly string[]> = {
  general: ['overlay-calibration', 'overlay-processing-style', 'reset-statistics'],
  recording: ['standby', 'standby-window', 'pre-roll', 'sound-cues', 'sound-volume', 'meeting-cues', 'smart-auto-stop'],
  text: ['developer-terms', 'project-scan', 'smart-correction', 'fuzzy-correction', 'homophone-context', 'smart-formatting', 'voice-commands'],
  delivery: ['app-overrides'],
  meetings: ['meeting-retention', 'meeting-echo-cancellation'],
  'ai-transcription': ['model-idle-timeout'],
  'ai-query': ['query-timeout'],
  'ai-transform': ['transform-practice'],
};

export function visibleSettingsTarget(target: string, settings: Settings): string {
  if (['paste-delay', 'smart-destination'].includes(target) && !settings.autoPaste) return 'auto-paste';
  if (target === 'paste-delay' && (settings.saveTranscript || settings.saveAudio)) return 'auto-paste';
  if (['cleanup-filler', 'cleanup-capitalize'].includes(target) && !settings.cleanupEnabled) return 'cleanup';
  if (target === 'project-scan' && !settings.codeVocabEnabled) return 'developer-terms';
  if (['fuzzy-correction', 'homophone-context'].includes(target) && !settings.correctionEnabled) return 'smart-correction';
  if (['standby-window', 'pre-roll'].includes(target) && !settings.captureStandbyEnabled) return 'standby';
  if (['sound-volume', 'meeting-cues'].includes(target) && !settings.soundCuesEnabled) return 'sound-cues';
  if (target === 'query-key' && settings.queryHotkey === null) return 'voice-query-shortcut';
  if (target === 'transform-key' && settings.transformHoldKey === null) return 'transform-shortcut';
  if (target === 'hotkey-feedback' && settings.recordingMode === 'hold_down') return 'recording-trigger';
  if (target === 'browser-site-rules' && !settings.siteModeLookupEnabled) return 'browser-sites';
  if (target === 'meeting-calendar' && !settings.meetingSuggestionsEnabled) return 'meeting-suggestions';
  return target;
}
