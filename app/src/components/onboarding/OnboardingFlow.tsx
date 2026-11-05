import { AppSurface } from '../ui/AppSurface';
import { showMainWindow } from '../../lib/windows';
import { defaultButtonKeyDown } from '../../lib/defaultButton';
import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Accessibility, AudioLines, CalendarDays, Check, CircleAlert, Cpu, Keyboard, LoaderCircle, Mic, Minus, Sparkles } from 'lucide-react';
import { Select } from '../ui/Select';
import { Button } from '../ui/FormControls';
import {
  checkAccessibilityPermission,
  checkMicrophonePermissionStatus,
  openMicrophoneSettings,
  requestAccessibilityPermission,
  requestMicrophoneAccess,
  resetAccessibilityPermission,
  resetMicrophonePermission,
  type MicPermissionStatus,
} from '../../lib/dictation';
import { preferredInstalledModel, useInstalledModels } from '../../lib/hooks/useInstalledModels';
import { ModelDownloadPanel } from '../ModelDownloader';
import { WindowHeader } from '../ui/WindowHeader';
import {
  DICTATION_KEY_OPTION_GROUPS,
  dictationKeyLabel,
  isFunctionDictationKey,
  type DictationKey,
  type ModelOption,
  type RecordingMode,
} from '../../lib/settings';
import {
  getSystemAudioPermissionStatus,
  openSystemAudioPreferences,
  requestSystemAudioPermission,
  type SystemAudioPermissionState,
} from '../../lib/meetings';
import {
  getCalendarPermissionStatus,
  openCalendarPreferences,
  requestCalendarPermission,
  resetCalendarPermission,
  type CalendarPermissionState,
} from '../../lib/calendar';

type Step =
  | 'welcome'
  | 'microphone'
  | 'accessibility'
  | 'systemAudio'
  | 'calendar'
  | 'model'
  | 'hotkey'
  | 'done';

const STEP_ORDER: Step[] = [
  'welcome',
  'microphone',
  'accessibility',
  'systemAudio',
  'calendar',
  'model',
  'hotkey',
  'done',
];

/** Longest the microphone step waits for an answer before re-enabling its actions. */
const MIC_PROMPT_WAIT_MS = 15_000;

interface Props {
  initialModel: ModelOption;
  /** Configured recording trigger, so the final tip shows the real binding. */
  recordingMode: RecordingMode;
  triggerKey: DictationKey;
  /** Called when the user finishes the wizard with the selected local setup. */
  onComplete: (model: ModelOption, recordingMode: RecordingMode, triggerKey: DictationKey) => void | Promise<void>;
}

/**
 * First-launch setup assistant.
 *
 * Walks a new install through core macOS permissions, optional meeting
 * permissions, and the model download. This replaces the old flow where the
 * mic TCC prompt only fired on the first recording attempt and permissions
 * were a dismissible banner.
 *
 * Microphone and Accessibility state is polled every second, plus on window
 * focus, so a grant made in System Settings flips the step live when the user
 * comes back. Optional meeting permissions are read only on their own steps.
 * The core permission steps handle the "wishy-washy" TCC states explicitly:
 * - mic `notDetermined`/`unknown` → in-app native prompt (request_microphone_access)
 * - mic `denied` → open System Settings, or reset the stale TCC entry, which
 *   returns the status to `notDetermined` so the in-app prompt works again
 * - accessibility listed-but-stale → reset entry + re-grant manually
 */
export function OnboardingFlow({ initialModel, recordingMode, triggerKey, onComplete }: Props) {
  const [step, setStep] = useState<Step>('welcome');
  const [actionTarget, setActionTarget] = useState<HTMLElement | null>(null);
  const stepContentRef = useRef<HTMLDivElement>(null);
  const previousStepRef = useRef<Step | null>(null);
  const activeStep = useRef<Step>(step);
  const [direction, setDirection] = useState<'forward' | 'back'>('forward');
  // Permission observations are tracked outside rendering: entry probes establish
  // a baseline, while requests and later polls can announce a fresh grant.
  const [freshGrant, setFreshGrant] = useState<Step | null>(null);
  const advanceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const permissionGrants = useRef<Partial<Record<Step, boolean>>>({});
  // Bumped on every navigation so probes started before a visit can't mark a
  // grant as fresh on the step being entered.
  const visitSeq = useRef(0);
  const micSettingsOpened = useRef(false);
  const micAutoRequested = useRef(false);
  const micPromptPending = useRef(false);
  const micWaitTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [micWaiting, setMicWaiting] = useState(false);
  // Ends the dialog wait once macOS answers, or after a bounded timeout when no
  // prompt appeared (for example a dev session whose terminal owns the prompt).
  const endMicWait = useCallback(() => {
    if (micWaitTimer.current !== null) clearTimeout(micWaitTimer.current);
    micWaitTimer.current = null;
    micPromptPending.current = false;
    micPending.current = false;
    setMicWaiting(false);
  }, []);
  useEffect(() => () => {
    if (micWaitTimer.current !== null) clearTimeout(micWaitTimer.current);
  }, []);

  const observePermission = useCallback((permission: Step, granted: boolean, baseline = false) => {
    const previous = permissionGrants.current[permission];
    permissionGrants.current[permission] = granted;
    if (activeStep.current === permission && !granted) setFreshGrant(null);
    if (baseline || previous !== false || !granted || activeStep.current !== permission) return;
    setFreshGrant(permission);
    if ((permission === 'accessibility' || (permission === 'microphone' && micSettingsOpened.current)) && !document.hasFocus()) {
      // A failed foreground request must not prevent navigation or discard the grant.
      void showMainWindow().catch(() => {});
    }
  }, []);
  const isDev = import.meta.env.DEV;
  const permissionOwner = isDev ? 'your terminal app' : 'Murmur';

  const [micStatus, setMicStatus] = useState<MicPermissionStatus>('unknown');
  const [micBusy, setMicBusy] = useState(false);
  const micPending = useRef(false);
  const [axBusy, setAxBusy] = useState(false);
  const axPending = useRef(false);
  const [micError, setMicError] = useState<string | null>(null);
  const [axGranted, setAxGranted] = useState<boolean | null>(null);
  const [axRequested, setAxRequested] = useState(false);
  const [axError, setAxError] = useState<string | null>(null);
  const [systemAudioStatus, setSystemAudioStatus] = useState<SystemAudioPermissionState>('unknown');
  const [systemAudioBusy, setSystemAudioBusy] = useState(false);
  const systemAudioPending = useRef(false);
  const [systemAudioError, setSystemAudioError] = useState<string | null>(null);
  const [calendarStatus, setCalendarStatus] = useState<CalendarPermissionState>('notDetermined');
  const [calendarBusy, setCalendarBusy] = useState(false);
  const calendarRequestPending = useRef(false);
  const [calendarError, setCalendarError] = useState<string | null>(null);
  const { installedModels, error: modelCatalogError, retry: retryModelCatalog } = useInstalledModels(step === 'model');
  // Whether the model step finished with a model on disk (Continue or download);
  // drives the done-step summary row.
  const [modelInstalled, setModelInstalled] = useState(false);
  const [installedModel, setInstalledModel] = useState<ModelOption>(initialModel);
  const [selectedRecordingMode, setSelectedRecordingMode] = useState(recordingMode);
  const [selectedTriggerKey, setSelectedTriggerKey] = useState(triggerKey);
  const [completing, setCompleting] = useState(false);
  const [completionError, setCompletionError] = useState(false);
  const completionPending = useRef(false);
  // Cancel in the model panel confirms native termination before releasing Back.
  const [modelDownloading, setModelDownloading] = useState(false);

  useEffect(() => {
    // Focus Welcome once (also under StrictMode), then each new step heading.
    // This puts Return in the default scope; permission polling never moves focus.
    if (previousStepRef.current === step) return;
    previousStepRef.current = step;
    stepContentRef.current?.querySelector('h1')?.focus();
  }, [step]);

  // Monotonic sequence so an interval probe overlapping a focus probe can't
  // apply an older TCC result over a newer one.
  const pollSeq = useRef(0);
  const systemAudioProbeSeq = useRef(0);
  const calendarProbeSeq = useRef(0);
  const calendarMutationSeq = useRef(0);
  const refreshPermissions = useCallback(async () => {
    const seq = ++pollSeq.current;
    const visit = visitSeq.current;
    let mic: MicPermissionStatus = 'unknown';
    let ax: boolean | null = null;
    try {
      mic = await checkMicrophonePermissionStatus();
    } catch {
      mic = 'unknown';
    }
    try {
      ax = await checkAccessibilityPermission();
    } catch {
      // keep previous value; a probe glitch must not flip the UI
    }
    if (seq !== pollSeq.current) return; // superseded by a newer probe
    const current = visit === visitSeq.current;
    if (current) observePermission('microphone', mic === 'granted');
    setMicStatus(mic);
    // request_microphone_access is fire-and-forget. Only a resolved TCC status
    // ends the dialog wait; resolving the IPC promise does not mean an answer.
    if (micPromptPending.current && mic !== 'notDetermined') endMicWait();
    if (ax !== null) {
      if (current) observePermission('accessibility', ax);
      setAxGranted(ax);
    }
  }, [observePermission, endMicWait]);

  useEffect(() => {
    refreshPermissions();
    const id = setInterval(refreshPermissions, 1000);
    window.addEventListener('focus', refreshPermissions);
    return () => {
      ++pollSeq.current;
      clearInterval(id);
      window.removeEventListener('focus', refreshPermissions);
    };
  }, [refreshPermissions]);

  useEffect(() => {
    if (step !== 'systemAudio') return;
    const seq = ++systemAudioProbeSeq.current;
    void getSystemAudioPermissionStatus().then((status) => {
      if (seq === systemAudioProbeSeq.current) {
        observePermission('systemAudio', status === 'granted', true);
        setSystemAudioStatus(status);
      }
    }).catch(() => {});
    return () => { ++systemAudioProbeSeq.current; };
  }, [step, observePermission]);

  // Calendar access is optional. Keep even its passive permission probe out of
  // the welcome and unrelated setup steps so onboarding never touches EventKit
  // before the user reaches this screen.
  useEffect(() => {
    if (step !== 'calendar') return;
    let stale = false;
    const refreshCalendarPermission = (baseline = false) => {
      const seq = ++calendarProbeSeq.current;
      setCalendarError(null);
      void getCalendarPermissionStatus().then(
        (status) => {
          if (!stale && seq === calendarProbeSeq.current) {
            observePermission('calendar', status === 'granted', baseline);
            setCalendarStatus(status);
          }
        },
        () => {
          if (!stale && seq === calendarProbeSeq.current) {
            setCalendarError('Could not check Calendar access. You can skip this step and name meetings manually.');
          }
        },
      );
    };

    refreshCalendarPermission(true);
    const onFocus = () => refreshCalendarPermission();
    window.addEventListener('focus', onFocus);
    return () => {
      stale = true;
      window.removeEventListener('focus', onFocus);
    };
  }, [step, observePermission]);

  const preferredModel = modelInstalled
    ? installedModel
    : preferredInstalledModel(initialModel, installedModels);

  const finishSetup = async () => {
    if (completionPending.current) return;
    completionPending.current = true;
    setCompleting(true);
    setCompletionError(false);
    try {
      await onComplete(installedModel, selectedRecordingMode, selectedTriggerKey);
    } catch {
      setCompletionError(true);
    } finally {
      completionPending.current = false;
      setCompleting(false);
    }
  };

  const stepIndex = STEP_ORDER.indexOf(step);
  const navigate = useCallback((next: Step, backwards = false) => {
    if (advanceTimer.current !== null) clearTimeout(advanceTimer.current);
    advanceTimer.current = null;
    // Leaving Microphone consumes its one automatic request, even when the
    // settle timer had not fired yet.
    if (activeStep.current === 'microphone') micAutoRequested.current = true;
    activeStep.current = next;
    ++visitSeq.current;
    // The first probe of each visit is the baseline; only later changes are fresh.
    delete permissionGrants.current[next];
    setFreshGrant(null);
    setDirection(backwards ? 'back' : 'forward');
    setStep(next);
    // Probe at entry so the visit's baseline lands before the user can act.
    if (next === 'microphone' || next === 'accessibility') void refreshPermissions();
  }, [refreshPermissions]);
  const goNext = () => navigate(STEP_ORDER[Math.min(stepIndex + 1, STEP_ORDER.length - 1)]);
  const goBack = () => navigate(STEP_ORDER[Math.max(stepIndex - 1, 0)], true);

  useEffect(() => {
    if (freshGrant !== step) return;
    advanceTimer.current = setTimeout(() => {
      if (activeStep.current === step) navigate(STEP_ORDER[STEP_ORDER.indexOf(step) + 1]);
    }, 900);
    return () => {
      if (advanceTimer.current !== null) clearTimeout(advanceTimer.current);
      advanceTimer.current = null;
    };
  }, [freshGrant, step, navigate]);

  const micGranted = micStatus === 'granted';
  const micDenied = micStatus === 'denied';

  const handleAllowMic = useCallback(async () => {
    if (micPending.current) return;
    micPending.current = true;
    micAutoRequested.current = true;
    permissionGrants.current.microphone ??= false;
    micPromptPending.current = micStatus === 'notDetermined';
    setMicWaiting(micPromptPending.current);
    setMicBusy(true);
    setMicError(null);
    try {
      // Fires the native TCC dialog when the status is notDetermined; the
      // 1s poll picks up the answer.
      await requestMicrophoneAccess();
    } catch (error) {
      micPromptPending.current = false;
      setMicWaiting(false);
      setMicError(typeof error === 'string' ? error : 'Could not request microphone access.');
    } finally {
      if (micPromptPending.current) {
        micWaitTimer.current = setTimeout(endMicWait, MIC_PROMPT_WAIT_MS);
      } else {
        micPending.current = false;
      }
      setMicBusy(false);
    }
  }, [micStatus, endMicWait]);

  useEffect(() => {
    if (step !== 'microphone' || micStatus !== 'notDetermined' || micAutoRequested.current) return;
    const timer = setTimeout(() => {
      if (!micAutoRequested.current && activeStep.current === 'microphone') void handleAllowMic();
    }, 600);
    return () => clearTimeout(timer);
  }, [step, micStatus, handleAllowMic]);

  const handleOpenMicSettings = async () => {
    setMicError(null);
    try {
      micSettingsOpened.current = true;
      permissionGrants.current.microphone ??= false;
      await openMicrophoneSettings();
    } catch (error) {
      setMicError(typeof error === 'string' ? error : 'Could not open System Settings.');
    }
  };

  const handleResetMic = async () => {
    setMicError(null);
    try {
      micSettingsOpened.current = true;
      permissionGrants.current.microphone ??= false;
      await resetMicrophonePermission();
      // After a reset the status returns to notDetermined, so the in-app
      // prompt button works again.
      endMicWait();
    } catch (error) {
      setMicError(
        typeof error === 'string'
          ? error
          : "Couldn't reset the Microphone entry. Check the logs for details.",
      );
    } finally {
      refreshPermissions();
    }
  };

  const handleGrantAx = async () => {
    if (axPending.current) return;
    axPending.current = true;
    permissionGrants.current.accessibility ??= false;
    setAxBusy(true);
    setAxError(null);
    setAxRequested(true);
    try {
      // Registers Murmur in the Accessibility list, shows the system dialog,
      // and opens the Accessibility pane.
      await requestAccessibilityPermission();
    } catch (error) {
      setAxError(typeof error === 'string' ? error : 'Could not open System Settings.');
    } finally {
      axPending.current = false;
      setAxBusy(false);
    }
  };

  const handleResetAx = async () => {
    setAxError(null);
    try {
      await resetAccessibilityPermission();
      setAxRequested(false);
    } catch (error) {
      setAxError(
        typeof error === 'string'
          ? error
          : "Couldn't reset the Accessibility entry. Check the logs for details.",
      );
    } finally {
      refreshPermissions();
    }
  };

  const handleRequestSystemAudio = async () => {
    if (systemAudioPending.current) return;
    systemAudioPending.current = true;
    permissionGrants.current.systemAudio ??= false;
    ++systemAudioProbeSeq.current;
    setSystemAudioError(null);
    setSystemAudioBusy(true);
    try {
      const access = await requestSystemAudioPermission();
      observePermission('systemAudio', access.permission === 'granted');
      setSystemAudioStatus(access.permission);
      if (access.needsRelaunch) {
        setSystemAudioError(
          'macOS lists Murmur as allowed, but the permission has not reached this session yet. Quit and reopen Murmur, then check again.',
        );
      }
    } catch (error) {
      setSystemAudioError(typeof error === 'string' ? error : 'Could not check System Audio access.');
    } finally {
      systemAudioPending.current = false;
      setSystemAudioBusy(false);
    }
  };

  const handleRequestCalendar = async () => {
    if (calendarRequestPending.current) return;
    calendarRequestPending.current = true;
    permissionGrants.current.calendar ??= false;
    const mutationSeq = ++calendarMutationSeq.current;
    ++calendarProbeSeq.current;
    setCalendarError(null);
    setCalendarBusy(true);
    try {
      const status = await requestCalendarPermission();
      if (mutationSeq === calendarMutationSeq.current) {
        ++calendarProbeSeq.current;
        observePermission('calendar', status === 'granted');
        setCalendarStatus(status);
      }
    } catch {
      if (mutationSeq === calendarMutationSeq.current) {
        setCalendarError('Could not request Calendar access. You can skip this step and name meetings manually.');
      }
    } finally {
      calendarRequestPending.current = false;
      if (mutationSeq === calendarMutationSeq.current) setCalendarBusy(false);
    }
  };

  const handleOpenCalendarSettings = async () => {
    setCalendarError(null);
    try {
      await openCalendarPreferences();
    } catch {
      setCalendarError('Could not open Calendar settings. Open Privacy & Security → Calendars in System Settings.');
    }
  };

  const handleResetCalendar = async () => {
    const mutationSeq = ++calendarMutationSeq.current;
    ++calendarProbeSeq.current;
    setCalendarError(null);
    setCalendarBusy(true);
    try {
      await resetCalendarPermission();
      if (mutationSeq === calendarMutationSeq.current) {
        ++calendarProbeSeq.current;
        observePermission('calendar', false);
        setCalendarStatus('notDetermined');
      }
    } catch {
      if (mutationSeq === calendarMutationSeq.current) {
        setCalendarError('Could not reset Calendar access. Open Privacy & Security → Calendars in System Settings.');
      }
    } finally {
      if (mutationSeq === calendarMutationSeq.current) setCalendarBusy(false);
    }
  };

  const permissionStatusMessage = step === 'microphone'
    ? `Microphone access ${micGranted ? 'granted' : micDenied ? 'denied' : micWaiting ? 'waiting for your answer in the macOS dialog' : 'not yet granted'}.`
    : step === 'accessibility'
    ? `Accessibility access ${axGranted === true ? 'granted' : axGranted === null ? 'being checked' : 'not granted'}.`
    : step === 'systemAudio'
    ? `System audio access ${systemAudioBusy ? 'waiting for macOS' : systemAudioStatus}.`
    : step === 'calendar'
    ? `Calendar access ${calendarBusy ? 'waiting for macOS' : calendarStatus === 'notDetermined' ? 'not yet requested' : calendarStatus}.`
    : '';

  return (
    <AppSurface data-default-scope onKeyDown={defaultButtonKeyDown} surface="onboarding" className="flex h-full min-h-0 flex-col overflow-hidden bg-background">
      <WindowHeader />
      <div className="onboarding-outer">
        <div ref={stepContentRef} className="onboarding-frame">
          <p role="status" aria-atomic="true" className="sr-only">{permissionStatusMessage}</p>
          <div className="onboarding-progress-wrap">
            <div className="onboarding-progress" role="progressbar" aria-label={`Step ${stepIndex + 1} of ${STEP_ORDER.length}`} aria-valuemin={1} aria-valuemax={STEP_ORDER.length} aria-valuenow={stepIndex + 1}>
              {STEP_ORDER.map((s, i) => <span key={s} aria-hidden="true" data-state={i === stepIndex ? 'current' : i < stepIndex ? 'complete' : 'upcoming'} />)}
            </div>
            <span>Step {stepIndex + 1} of {STEP_ORDER.length}</span>
          </div>
          <ActionTarget.Provider value={actionTarget}>
          <div className="onboarding-content" data-direction={direction} key={step}>
            {step === 'welcome' && <>
              <StepHeading icon={Sparkles} title="Welcome to Murmur" subtitle="Dictation runs on your Mac. Saved history and retained meeting audio stay local." />
              <p className="onboarding-copy">Setup covers permissions, an offline transcription model, and your recording shortcut.</p>
              <WizardNavigationRow onBack={goBack} hideBack onNext={goNext} nextEnabled defaultAction nextLabel="Get started" />
            </>}
            {step === 'microphone' && <>
              <StepHeading icon={Mic} title="Microphone access" subtitle="Murmur records your speech for local transcription. Audio is discarded after transcription." />
              <PermissionCard icon={Mic} label="Microphone" state={micGranted ? 'granted' : micDenied ? 'denied' : 'notSet'} waiting={micBusy || micWaiting} fresh={freshGrant === step} />
              {micWaiting && <p className="onboarding-copy">Waiting for your answer in the macOS dialog…</p>}
              {micDenied && <><p className="onboarding-copy">Enable {permissionOwner} under Privacy &amp; Security → Microphone, then come back. This screen updates automatically.</p>{isDev ? <DevPermissionHelp /> : <button className="onboarding-link" onClick={handleResetMic}>Reset permission…</button>}</>}
              {micError && <ErrorMessage message={micError} />}
              <WizardNavigationRow onBack={goBack} onNext={micGranted ? goNext : micDenied ? handleOpenMicSettings : handleAllowMic} onSkip={goNext} nextEnabled={!micBusy && !micWaiting} defaultAction={micGranted} nextLabel={micDenied ? 'Open System Settings…' : 'Continue'} skippable={!micGranted && !micWaiting && !micBusy} skipLabel="Skip for now" />
            </>}
            {step === 'accessibility' && <>
              <StepHeading icon={Accessibility} title="Accessibility access" subtitle="Use the global recording key and auto-paste while Murmur is in the background." />
              <PermissionCard icon={Accessibility} label="Accessibility" state={axGranted ? 'granted' : 'notSet'} waiting={axBusy || axGranted === null} fresh={freshGrant === step} />
              {!axGranted && <><p className="onboarding-copy">Turn on {permissionOwner} in System Settings, then come back. This screen updates automatically.</p>{axRequested && (isDev ? <DevPermissionHelp /> : <button className="onboarding-link" onClick={handleResetAx}>Murmur is listed but still not detected? Reset permission…</button>)}</>}
              {axError && <ErrorMessage message={axError} />}
              <WizardNavigationRow onBack={goBack} onNext={axGranted ? goNext : handleGrantAx} onSkip={goNext} nextEnabled={!axBusy} defaultAction={axGranted === true} nextLabel={axGranted ? 'Continue' : 'Open System Settings…'} skippable={!axGranted} skipLabel="Skip for now" />
            </>}
            {step === 'systemAudio' && <>
              <StepHeading icon={AudioLines} title="System audio access" subtitle="Optional for meetings. Capture audio from your Mac alongside your microphone." />
              <PermissionCard icon={AudioLines} label="System audio" state={systemAudioStatus} waiting={systemAudioBusy} fresh={freshGrant === step} />
              {systemAudioStatus === 'unknown' && <p className="onboarding-copy">Continue shows the macOS permission prompt.</p>}
              <p className="onboarding-copy">{systemAudioStatus === 'unsupported' ? 'Meeting capture requires macOS 14.2 or newer. Dictation is still available.' : 'macOS may list Murmur under Privacy & Security → Screen & System Audio Recording.'}</p>
              {(systemAudioStatus === 'denied' || systemAudioStatus === 'granted') && <button className="onboarding-link" disabled={systemAudioBusy} onClick={() => void handleRequestSystemAudio()}>{systemAudioStatus === 'denied' ? 'Check again' : 'Re-check system audio access'}</button>}
              {systemAudioError && <ErrorMessage message={systemAudioError} />}
              <WizardNavigationRow onBack={goBack} onNext={systemAudioStatus === 'denied' ? () => void openSystemAudioPreferences() : systemAudioStatus === 'unknown' ? handleRequestSystemAudio : goNext} onSkip={goNext} nextEnabled={!systemAudioBusy} defaultAction={systemAudioStatus !== 'denied' && systemAudioStatus !== 'unknown'} nextLabel={systemAudioStatus === 'denied' ? 'Open System Settings…' : 'Continue'} skippable={systemAudioStatus !== 'granted'} skipLabel="Skip meetings for now" />
            </>}
            {step === 'calendar' && <>
              <StepHeading icon={CalendarDays} title="Calendar access" subtitle="Optional for meetings. Offer event names and attendees for you to confirm, without changing your calendar." />
              <PermissionCard icon={CalendarDays} label="Calendar" state={calendarStatus} waiting={calendarBusy} fresh={freshGrant === step} />
              {calendarStatus === 'notDetermined' && <p className="onboarding-copy">Continue shows the macOS permission prompt.</p>}
              <p className="onboarding-copy">{calendarStatus === 'denied' ? `Enable ${permissionOwner} under Privacy & Security → Calendars, or name meetings manually.` : calendarStatus === 'restricted' || calendarStatus === 'unsupported' ? 'You can still name meetings manually.' : 'Calendar details stay on this Mac.'}</p>
              {calendarStatus === 'denied' && (isDev ? <DevPermissionHelp /> : <button className="onboarding-link" disabled={calendarBusy} onClick={handleResetCalendar}>Reset Calendar permission…</button>)}
              {calendarError && <ErrorMessage message={calendarError} />}
              <WizardNavigationRow onBack={goBack} onNext={calendarStatus === 'denied' ? handleOpenCalendarSettings : calendarStatus === 'notDetermined' ? handleRequestCalendar : goNext} onSkip={goNext} nextEnabled={!calendarBusy} defaultAction={calendarStatus !== 'denied' && calendarStatus !== 'notDetermined'} nextLabel={calendarStatus === 'denied' ? 'Open System Settings…' : 'Continue'} skippable={calendarStatus !== 'granted'} skipLabel="Skip Calendar for now" />
            </>}
            {step === 'model' && <>
              <StepHeading icon={Cpu} title="Transcription model" subtitle="Download a local model once, then transcribe offline." />
              {modelCatalogError ? <><ErrorMessage message="Could not check installed models. Retry to continue setup." /><WizardNavigationRow onBack={goBack} onNext={retryModelCatalog} nextEnabled nextLabel="Retry" /></> : installedModels === null ? <WizardNavigationRow onBack={goBack} onNext={() => {}} nextEnabled={false} nextLabel="Loading…" /> : <ModelDownloadPanel initialModel={preferredModel} installedModels={installedModels} onDownloadingChange={setModelDownloading} renderPrimaryAction={({ label, disabled, onActivate, isNavigation }) => <WizardNavigationRow onBack={goBack} backDisabled={modelDownloading || disabled} onNext={onActivate} nextEnabled={!disabled} defaultAction={isNavigation} nextLabel={label} />} onComplete={(model) => { setInstalledModel(model); setModelInstalled(true); setModelDownloading(false); goNext(); }} />}
            </>}
            {step === 'hotkey' && <>
              <StepHeading icon={Keyboard} title="Recording shortcut" subtitle="Choose how Murmur listens. You can change both options later in Recording settings." />
              <div className="dialog-card onboarding-shortcut">
                <p className="onboarding-label">Recording trigger</p>
                <div role="group" aria-label="Recording trigger" className="settings-segmented onboarding-segmented">
                  {([['hold_down', 'Hold down'], ['double_tap', 'Double-tap'], ['both', 'Both']] as const).map(([value, label]) => <button key={value} type="button" aria-pressed={selectedRecordingMode === value} data-selected={selectedRecordingMode === value} onClick={() => setSelectedRecordingMode(value)} className="settings-segmented-btn">{label}</button>)}
                </div>
                <p id="onboarding-trigger-key-label" className="onboarding-label">Trigger key</p>
                <Select aria-labelledby="onboarding-trigger-key-label" value={selectedTriggerKey} onChange={setSelectedTriggerKey} items={DICTATION_KEY_OPTION_GROUPS} />
                {isFunctionDictationKey(selectedTriggerKey) && <p className="onboarding-copy">On Apple keyboards, F1–F12 may require Fn or the macOS setting that uses function keys as standard keys. Murmur does not suppress the key's normal system or app action.</p>}
              </div>
              <WizardNavigationRow onBack={goBack} onNext={goNext} nextEnabled defaultAction nextLabel="Continue" />
            </>}
            {step === 'done' && <>
              <StepHeading icon={Check} title="You're all set" subtitle="Here's how everything looks." />
              <div className="dialog-card onboarding-summary">
                <SummaryRow ok={micGranted} label="Microphone" okText="Granted" missingText="Not granted — enable later in Settings" />
                <SummaryRow ok={axGranted === true} label="Accessibility" okText="Granted" missingText="Not granted — shortcut unavailable outside Murmur" />
                <SummaryRow ok={systemAudioStatus === 'granted'} label="System audio" okText="Granted" missingText="Optional — enable later in Meetings" />
                <SummaryRow ok={calendarStatus === 'granted'} label="Calendar" okText="Granted" missingText="Optional — name meetings manually" />
                <SummaryRow ok={modelInstalled} label="Model" okText="Installed" missingText="Not verified" />
              </div>
              <p className="onboarding-label">Try it out</p><p className="onboarding-copy">{selectedRecordingMode === 'double_tap' ? 'Double-tap ' : 'Hold '}<kbd className="dialog-kbd">{dictationKeyLabel(selectedTriggerKey)}</kbd>{selectedRecordingMode === 'double_tap' ? ' to start recording and tap once to stop' : selectedRecordingMode === 'both' ? ' and speak, then release (or double-tap to toggle)' : ' and speak, then release'}. Your words are transcribed and copied to the clipboard.</p>
              {completionError && <ErrorMessage message="Could not verify or configure the selected transcription model. Your setup choices are kept. Retry, or go Back to review the model." />}
              <WizardNavigationRow onBack={goBack} backDisabled={completing} onNext={() => void finishSetup()} nextEnabled={!completing} defaultAction nextLabel={completing ? 'Configuring model…' : completionError ? 'Retry' : 'Start using Murmur'} />
            </>}
          </div>
          </ActionTarget.Provider>
          <div className="onboarding-actions" ref={setActionTarget} />
        </div>
      </div>
    </AppSurface>
  );
}

const ActionTarget = createContext<HTMLElement | null>(null);

function DevPermissionHelp() {
  return <p className="onboarding-copy">In this development session, grant access to the terminal app that launched Murmur, then restart the dev server. If access stays stale, turn that terminal's permission off and on in System Settings.</p>;
}

function StepHeading({ icon: Icon, title, subtitle }: { icon: typeof Mic; title: string; subtitle: string }) {
  return <header className="onboarding-heading"><div className="onboarding-icon"><Icon aria-hidden="true" /></div><h1 tabIndex={-1}>{title}</h1><p>{subtitle}</p></header>;
}

function PermissionCard({ icon: Icon, label, state, waiting = false, fresh = false }: { icon: typeof Mic; label: string; state: string; waiting?: boolean; fresh?: boolean }) {
  const granted = state === 'granted';
  const denied = state === 'denied';
  const StatusIcon = granted ? Check : waiting ? LoaderCircle : denied || state === 'restricted' ? CircleAlert : Minus;
  const text = granted ? 'Allowed' : waiting ? 'Waiting for macOS…' : denied ? 'Denied' : state === 'restricted' ? 'Restricted' : state === 'unsupported' ? 'Unavailable' : 'Not set';
  return <div className="dialog-card onboarding-permission-card">
    <Icon aria-hidden="true" size={20} />
    <span className="onboarding-permission-label">{label}</span>
    <span className="onboarding-permission-state" data-tone={granted ? 'success' : denied ? 'error' : 'neutral'} data-fresh={granted && fresh}>
      <StatusIcon aria-hidden="true" size={16} className={waiting && !granted ? 'onboarding-spinner' : undefined} />{text}
    </span>
  </div>;
}

function ErrorMessage({ message }: { message: string }) {
  return <div role="alert" className="onboarding-error"><div className="onboarding-status" data-tone="error"><CircleAlert aria-hidden="true" size={16} /><span>Something went wrong</span></div><p className="onboarding-copy">{message}</p></div>;
}

function SummaryRow({ ok, label, okText, missingText }: { ok: boolean; label: string; okText: string; missingText: string }) {
  return <div className="onboarding-summary-row"><span aria-hidden="true" className={ok ? 'text-success' : 'text-on-surface-variant'}>{ok ? <Check size={16} /> : <Minus size={16} />}</span><strong>{label}</strong><span>{ok ? okText : missingText}</span></div>;
}

function WizardNavigationRow({ onBack, backDisabled = false, hideBack = false, onNext, onSkip, nextEnabled, nextLabel, defaultAction = false, skippable = false, skipLabel = 'Skip' }: { onBack: () => void; backDisabled?: boolean; hideBack?: boolean; onNext: () => void; onSkip?: () => void; nextEnabled: boolean; nextLabel: string; defaultAction?: boolean; skippable?: boolean; skipLabel?: string }) {
  const target = useContext(ActionTarget);
  if (!target) return null;
  return createPortal(<nav aria-label="Setup step actions" className="onboarding-nav">
    {!hideBack && <Button variant="ghost" onClick={onBack} disabled={backDisabled} aria-label="Go back to the previous setup step" title={backDisabled ? 'Please wait for the model download to finish' : undefined}>Back</Button>}
    <div role="group" aria-label="Step actions" className="onboarding-nav-end">
      {skippable && <Button variant="ghost" onClick={onSkip ?? onNext}>{skipLabel}</Button>}
      <Button variant="primary" onClick={onNext} data-default-button={defaultAction || undefined} disabled={!nextEnabled}>{nextLabel}</Button>
    </div>
  </nav>, target);
}
