import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  checkMicrophonePermissionStatus,
  resetAccessibilityPermission,
  resetMicrophonePermission,
  type MicPermissionStatus,
} from '../../lib/dictation';
import { ModelDownloadPanel } from '../ModelDownloader';
import type { DoubleTapKey, ModelOption, RecordingMode } from '../../lib/settings';

type Step = 'welcome' | 'microphone' | 'accessibility' | 'model' | 'done';

const STEP_ORDER: Step[] = ['welcome', 'microphone', 'accessibility', 'model', 'done'];

const KEY_LABELS: Record<DoubleTapKey, string> = {
  shift_l: 'Left Shift',
  alt_l: 'Left Option',
  ctrl_r: 'Right Control',
};

interface Props {
  initialModel: ModelOption;
  /** Configured recording trigger, so the final tip shows the real binding. */
  recordingMode: RecordingMode;
  triggerKey: DoubleTapKey;
  /** Called when the user finishes the wizard; receives the installed model. */
  onComplete: (model: ModelOption) => void;
}

/**
 * First-launch setup assistant.
 *
 * Walks a new install through the two macOS permissions and the model
 * download, replacing the old flow where the mic TCC prompt only fired on the
 * first recording attempt and permissions were a dismissible banner.
 *
 * Permission state is polled every second (plus on window focus) for the whole
 * wizard lifetime, so a grant made in System Settings flips the step live when
 * the user comes back. Both permission steps handle the "wishy-washy" TCC
 * states explicitly:
 * - mic `notDetermined`/`unknown` → in-app native prompt (request_microphone_access)
 * - mic `denied` → open System Settings, or reset the stale TCC entry, which
 *   returns the status to `notDetermined` so the in-app prompt works again
 * - accessibility listed-but-stale → reset entry + re-grant manually
 */
export function OnboardingFlow({ initialModel, recordingMode, triggerKey, onComplete }: Props) {
  const [step, setStep] = useState<Step>('welcome');
  const [micStatus, setMicStatus] = useState<MicPermissionStatus>('unknown');
  const [micRequested, setMicRequested] = useState(false);
  const [micError, setMicError] = useState<string | null>(null);
  const [axGranted, setAxGranted] = useState<boolean | null>(null);
  const [axRequested, setAxRequested] = useState(false);
  const [axError, setAxError] = useState<string | null>(null);
  // null = not checked yet; the model step shows a spinner-less blank until known.
  const [modelInstalled, setModelInstalled] = useState<boolean | null>(null);
  const [installedModel, setInstalledModel] = useState<ModelOption>(initialModel);
  // Lock Back while a download is in flight: unmounting the panel wouldn't stop
  // the Rust download_model command, and re-entering the step could start a
  // second concurrent download of the same file.
  const [modelDownloading, setModelDownloading] = useState(false);

  // Monotonic sequence so an interval probe overlapping a focus probe can't
  // apply an older TCC result over a newer one.
  const pollSeq = useRef(0);
  const refreshPermissions = useCallback(async () => {
    const seq = ++pollSeq.current;
    let mic: MicPermissionStatus = 'unknown';
    let ax: boolean | null = null;
    try {
      mic = await checkMicrophonePermissionStatus();
    } catch {
      mic = 'unknown';
    }
    try {
      ax = await invoke<boolean>('check_accessibility_permission');
    } catch {
      // keep previous value; a probe glitch must not flip the UI
    }
    if (seq !== pollSeq.current) return; // superseded by a newer probe
    setMicStatus(mic);
    if (ax !== null) setAxGranted(ax);
  }, []);

  useEffect(() => {
    refreshPermissions();
    const id = setInterval(refreshPermissions, 1000);
    window.addEventListener('focus', refreshPermissions);
    return () => {
      clearInterval(id);
      window.removeEventListener('focus', refreshPermissions);
    };
  }, [refreshPermissions]);

  // Check whether the selected model is already on disk when entering the
  // model step (re-run of the wizard, or a partially completed first launch).
  useEffect(() => {
    if (step !== 'model') return;
    invoke<boolean>('check_specific_model_exists', { modelName: initialModel })
      .then(setModelInstalled)
      // Fail open (skip the download) — matches the standalone gate's behavior;
      // a genuine fresh install resolves to `false` rather than throwing.
      .catch(() => setModelInstalled(true));
  }, [step, initialModel]);

  const stepIndex = STEP_ORDER.indexOf(step);
  const goNext = () => setStep(STEP_ORDER[Math.min(stepIndex + 1, STEP_ORDER.length - 1)]);
  const goBack = () => setStep(STEP_ORDER[Math.max(stepIndex - 1, 0)]);

  const micGranted = micStatus === 'granted';
  const micDenied = micStatus === 'denied';

  const handleAllowMic = async () => {
    setMicError(null);
    setMicRequested(true);
    try {
      // Fires the native TCC dialog when the status is notDetermined; the
      // 1s poll picks up the answer.
      await invoke('request_microphone_access');
    } catch (error) {
      setMicError(typeof error === 'string' ? error : 'Could not request microphone access.');
    }
  };

  const handleOpenMicSettings = async () => {
    setMicError(null);
    try {
      await invoke('request_microphone_permission');
    } catch (error) {
      setMicError(typeof error === 'string' ? error : 'Could not open System Settings.');
    }
  };

  const handleResetMic = async () => {
    setMicError(null);
    try {
      await resetMicrophonePermission();
      // After a reset the status returns to notDetermined, so the in-app
      // prompt button works again.
      setMicRequested(false);
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
    setAxError(null);
    setAxRequested(true);
    try {
      // Registers Murmur in the Accessibility list, shows the system dialog,
      // and opens the Accessibility pane.
      await invoke('request_accessibility_permission');
    } catch (error) {
      setAxError(typeof error === 'string' ? error : 'Could not open System Settings.');
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

  return (
    <div className="h-screen bg-stone-50 dark:bg-stone-900 flex flex-col items-center justify-center p-8 font-[-apple-system,BlinkMacSystemFont,'Segoe_UI',Roboto,sans-serif]">
      <div className="w-full max-w-md">
        {/* Progress dots */}
        <div className="flex items-center justify-center gap-2 mb-8" aria-label={`Step ${stepIndex + 1} of ${STEP_ORDER.length}`}>
          {STEP_ORDER.map((s, i) => (
            <span
              key={s}
              className={`h-1.5 rounded-full transition-all duration-300 ${
                i === stepIndex
                  ? 'w-6 bg-blue-500'
                  : i < stepIndex
                  ? 'w-1.5 bg-blue-400/60'
                  : 'w-1.5 bg-stone-300 dark:bg-stone-600'
              }`}
            />
          ))}
        </div>

        {step === 'welcome' && (
          <div className="text-center">
            <h1 className="text-2xl font-semibold text-stone-800 dark:text-stone-100 mb-2">
              Welcome to Murmur
            </h1>
            <p className="text-sm text-stone-500 dark:text-stone-400 mb-2">
              Voice-to-text that runs entirely on your Mac. No cloud, no accounts —
              your audio never leaves this machine.
            </p>
            <p className="text-sm text-stone-500 dark:text-stone-400 mb-8">
              Setup takes about a minute: two macOS permissions and a one-time
              model download.
            </p>
            <button
              onClick={goNext}
              className="w-full py-2.5 px-4 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-lg transition-colors"
            >
              Get Started
            </button>
          </div>
        )}

        {step === 'microphone' && (
          <div>
            <StepHeading
              title="Microphone Access"
              granted={micGranted}
              subtitle="Murmur records from your microphone to transcribe your speech. Audio is processed locally and discarded after transcription."
            />

            {micGranted ? (
              <GrantedCard label="Microphone access granted" />
            ) : micDenied ? (
              <div className="mb-6 px-4 py-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg space-y-3">
                <p className="text-sm text-red-700 dark:text-red-300">
                  Microphone access is denied. Enable Murmur under Privacy &amp;
                  Security → Microphone, then come back — this screen updates
                  automatically.
                </p>
                <button
                  onClick={handleOpenMicSettings}
                  className="w-full py-2 px-4 bg-red-600 hover:bg-red-700 text-white text-sm font-medium rounded-lg transition-colors"
                >
                  Open System Settings
                </button>
                <div>
                  <button
                    onClick={handleResetMic}
                    className="text-xs text-red-600/80 dark:text-red-400/80 underline hover:no-underline"
                  >
                    Still not working? Reset the permission
                  </button>
                  <p className="mt-1 text-xs text-red-600/70 dark:text-red-400/70">
                    Clears Murmur's stale Microphone entry so macOS can ask fresh —
                    useful when the toggle is on but recording still fails.
                  </p>
                </div>
              </div>
            ) : (
              <div className="mb-6">
                <button
                  onClick={handleAllowMic}
                  className="w-full py-2.5 px-4 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-lg transition-colors"
                >
                  Allow Microphone Access
                </button>
                {micRequested && (
                  <p className="mt-2 text-xs text-stone-500 dark:text-stone-400 text-center">
                    Waiting for your answer in the macOS dialog…
                  </p>
                )}
              </div>
            )}

            {micError && (
              <p className="mb-4 text-xs text-red-600 dark:text-red-400">{micError}</p>
            )}

            <WizardFooter
              onBack={goBack}
              onNext={goNext}
              nextEnabled={micGranted}
              nextLabel="Continue"
              skippable={!micGranted}
              skipLabel="Skip for now"
            />
          </div>
        )}

        {step === 'accessibility' && (
          <div>
            <StepHeading
              title="Accessibility Access"
              granted={axGranted === true}
              subtitle="Needed for the global recording key (so it works while Murmur is in the background) and for auto-paste. Without it, recording only works from buttons inside the app."
            />

            {axGranted ? (
              <GrantedCard label="Accessibility access granted" />
            ) : (
              <div className="mb-6 space-y-3">
                <button
                  onClick={handleGrantAx}
                  className="w-full py-2.5 px-4 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-lg transition-colors"
                >
                  Grant Accessibility Access
                </button>
                <p className="text-xs text-stone-500 dark:text-stone-400 text-center">
                  macOS opens System Settings — turn on <strong>Murmur</strong> in the
                  list, then come back. This screen updates automatically.
                </p>
                {axRequested && (
                  <div className="pt-1">
                    <button
                      onClick={handleResetAx}
                      className="text-xs text-stone-500/90 dark:text-stone-400/90 underline hover:no-underline"
                    >
                      Murmur is listed and enabled, but still not detected? Reset the permission
                    </button>
                    <p className="mt-1 text-xs text-stone-400 dark:text-stone-500">
                      Clears a stale Accessibility entry (common after reinstalling).
                      You'll need to re-enable Murmur in the list afterward.
                    </p>
                  </div>
                )}
              </div>
            )}

            {axError && (
              <p className="mb-4 text-xs text-red-600 dark:text-red-400">{axError}</p>
            )}

            <WizardFooter
              onBack={goBack}
              onNext={goNext}
              nextEnabled={axGranted === true}
              nextLabel="Continue"
              skippable={axGranted !== true}
              skipLabel="Skip for now"
            />
          </div>
        )}

        {step === 'model' && (
          <div>
            <h1 className="text-xl font-semibold text-stone-800 dark:text-stone-100 mb-1">
              Transcription Model
            </h1>
            <p className="text-sm text-stone-500 dark:text-stone-400 mb-6">
              Murmur transcribes with a local model — downloaded once, then everything
              runs offline.
            </p>

            {modelInstalled === null ? (
              <div className="h-24" />
            ) : modelInstalled ? (
              <div>
                <GrantedCard label="Model already installed" />
                <WizardFooter onBack={goBack} onNext={goNext} nextEnabled nextLabel="Continue" />
              </div>
            ) : (
              <div>
                <ModelDownloadPanel
                  initialModel={initialModel}
                  onDownloadingChange={setModelDownloading}
                  onComplete={(model) => {
                    setInstalledModel(model);
                    setModelInstalled(true);
                    setModelDownloading(false);
                    goNext();
                  }}
                />
                {!modelDownloading && (
                  <div className="mt-3 text-center">
                    <button
                      onClick={goBack}
                      className="text-xs text-stone-400 dark:text-stone-500 hover:text-stone-600 dark:hover:text-stone-300 transition-colors"
                    >
                      Back
                    </button>
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {step === 'done' && (
          <div>
            <h1 className="text-xl font-semibold text-stone-800 dark:text-stone-100 mb-1 text-center">
              You're all set
            </h1>
            <p className="text-sm text-stone-500 dark:text-stone-400 mb-6 text-center">
              Here's how everything looks:
            </p>

            <div className="space-y-2 mb-6">
              <SummaryRow ok={micGranted} label="Microphone" okText="Granted" missingText="Not granted — grant later from the in-app banner or Settings" />
              <SummaryRow ok={axGranted === true} label="Accessibility" okText="Granted" missingText="Not granted — the recording key won't work outside the app" />
              <SummaryRow ok={modelInstalled === true} label="Model" okText="Installed" missingText="Not verified — the app will ask again if it's missing" />
            </div>

            <div className="mb-6 px-4 py-3 bg-stone-100 dark:bg-stone-800 rounded-lg">
              <p className="text-sm text-stone-700 dark:text-stone-300 font-medium mb-1">
                Try it out
              </p>
              <p className="text-xs text-stone-500 dark:text-stone-400">
                {recordingMode === 'double_tap' ? 'Double-tap ' : 'Hold '}
                <kbd className="px-1 py-0.5 rounded bg-white dark:bg-stone-700 border border-stone-300 dark:border-stone-600 font-mono text-[10px]">{KEY_LABELS[triggerKey]}</kbd>
                {recordingMode === 'double_tap'
                  ? ' to start recording and tap it once to stop'
                  : recordingMode === 'both'
                  ? ' and speak, then release (or double-tap to toggle)'
                  : ' and speak, then release'}
                {' '}— your words are transcribed and copied to the clipboard. The
                recording key, auto-paste, and everything else can be changed in
                Settings.
              </p>
            </div>

            <button
              onClick={() => onComplete(installedModel)}
              className="w-full py-2.5 px-4 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-lg transition-colors"
            >
              Start Using Murmur
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function StepHeading({ title, subtitle, granted }: { title: string; subtitle: string; granted: boolean }) {
  return (
    <div className="mb-6">
      <div className="flex items-center gap-2 mb-1">
        <h1 className="text-xl font-semibold text-stone-800 dark:text-stone-100">{title}</h1>
        {granted && <CheckIcon />}
      </div>
      <p className="text-sm text-stone-500 dark:text-stone-400">{subtitle}</p>
    </div>
  );
}

function GrantedCard({ label }: { label: string }) {
  return (
    <div className="mb-6 px-4 py-3 bg-emerald-50 dark:bg-emerald-900/20 border border-emerald-200 dark:border-emerald-800 rounded-lg flex items-center gap-2">
      <CheckIcon />
      <span className="text-sm text-emerald-700 dark:text-emerald-300">{label}</span>
    </div>
  );
}

function SummaryRow({ ok, label, okText, missingText }: { ok: boolean; label: string; okText: string; missingText: string }) {
  return (
    <div className="flex items-start gap-2 px-4 py-2.5 bg-white dark:bg-stone-800 border border-stone-200 dark:border-stone-700 rounded-lg">
      <span className={`mt-1 w-2 h-2 shrink-0 rounded-full ${ok ? 'bg-emerald-500' : 'bg-amber-500'}`} />
      <div className="min-w-0">
        <span className="text-sm font-medium text-stone-700 dark:text-stone-200">{label}</span>
        <span className="text-sm text-stone-500 dark:text-stone-400"> — {ok ? okText : missingText}</span>
      </div>
    </div>
  );
}

function WizardFooter({
  onBack,
  onNext,
  nextEnabled,
  nextLabel,
  skippable = false,
  skipLabel = 'Skip',
}: {
  onBack: () => void;
  onNext: () => void;
  nextEnabled: boolean;
  nextLabel: string;
  skippable?: boolean;
  skipLabel?: string;
}) {
  return (
    <div className="flex items-center justify-between">
      <button
        onClick={onBack}
        className="text-xs text-stone-400 dark:text-stone-500 hover:text-stone-600 dark:hover:text-stone-300 transition-colors"
      >
        Back
      </button>
      <div className="flex items-center gap-3">
        {skippable && (
          <button
            onClick={onNext}
            className="text-xs text-stone-400 dark:text-stone-500 hover:text-stone-600 dark:hover:text-stone-300 transition-colors"
          >
            {skipLabel}
          </button>
        )}
        <button
          onClick={onNext}
          disabled={!nextEnabled}
          className="py-2 px-5 bg-blue-600 hover:bg-blue-700 disabled:opacity-40 disabled:cursor-not-allowed text-white text-sm font-medium rounded-lg transition-colors"
        >
          {nextLabel}
        </button>
      </div>
    </div>
  );
}

function CheckIcon() {
  return (
    <svg className="w-4 h-4 text-emerald-500 shrink-0" fill="none" stroke="currentColor" strokeWidth={2.5} viewBox="0 0 24 24">
      <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
    </svg>
  );
}
