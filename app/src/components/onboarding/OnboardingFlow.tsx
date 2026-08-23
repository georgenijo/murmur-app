import { useCallback, useEffect, useRef, useState } from 'react';
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
import { getModelRuntimeCatalog } from '../../lib/modelRuntime';
import { DOWNLOAD_MODEL_KEYS, ModelDownloadPanel } from '../ModelDownloader';
import { WindowHeader } from '../ui/WindowHeader';
import type { DoubleTapKey, ModelOption, RecordingMode } from '../../lib/settings';
import {
  getSystemAudioPermissionStatus,
  openSystemAudioPreferences,
  requestSystemAudioPermission,
  type SystemAudioPermissionState,
} from '../../lib/meetings';

type Step = 'welcome' | 'microphone' | 'accessibility' | 'systemAudio' | 'model' | 'hotkey' | 'done';

const STEP_ORDER: Step[] = ['welcome', 'microphone', 'accessibility', 'systemAudio', 'model', 'hotkey', 'done'];

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
  /** Called when the user finishes the wizard with the selected local setup. */
  onComplete: (model: ModelOption, recordingMode: RecordingMode, triggerKey: DoubleTapKey) => void;
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
  const [systemAudioStatus, setSystemAudioStatus] = useState<SystemAudioPermissionState>('unknown');
  const [systemAudioBusy, setSystemAudioBusy] = useState(false);
  const [systemAudioError, setSystemAudioError] = useState<string | null>(null);
  // Per-model on-disk status for every option the download panel offers.
  // null = not probed yet; the model step shows a spinner-less blank until known.
  const [installedModels, setInstalledModels] = useState<Partial<Record<ModelOption, boolean>> | null>(null);
  // Whether the model step finished with a model on disk (Continue or download);
  // drives the done-step summary row.
  const [modelInstalled, setModelInstalled] = useState(false);
  const [installedModel, setInstalledModel] = useState<ModelOption>(initialModel);
  const [selectedRecordingMode, setSelectedRecordingMode] = useState(recordingMode);
  const [selectedTriggerKey, setSelectedTriggerKey] = useState(triggerKey);
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
      ax = await checkAccessibilityPermission();
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

  // Probe every offered model when entering the model step (re-run of the
  // wizard, or a fresh webview data store next to an installed app), so any
  // already-downloaded model shows as Installed instead of just the settings
  // default (#240).
  useEffect(() => {
    if (step !== 'model') return;
    let stale = false;
    getModelRuntimeCatalog().then((catalog) => {
      if (!stale) {
        const byName = new Map(catalog.map((model) => [model.modelName, model]));
        const entries = DOWNLOAD_MODEL_KEYS.map((model) => [
          model,
          byName.get(model)?.installState === 'installed',
        ] as const);
        setInstalledModels(Object.fromEntries(entries) as Partial<Record<ModelOption, boolean>>);
      }
    }, () => {
      if (!stale) {
        // Preserve the previous fail-open onboarding behavior if the status
        // command itself is unavailable; genuine missing models are reported
        // as notInstalled rather than rejecting the request.
        setInstalledModels(Object.fromEntries(
          DOWNLOAD_MODEL_KEYS.map((model) => [model, true]),
        ) as Partial<Record<ModelOption, boolean>>);
      }
    });
    return () => {
      stale = true;
    };
  }, [step]);

  useEffect(() => {
    if (step !== 'systemAudio') return;
    void getSystemAudioPermissionStatus().then(setSystemAudioStatus).catch(() => {});
  }, [step]);

  // If the settings model is missing but another offered model is on disk,
  // pre-select an installed one so the primary action is Continue, not Download.
  const preferredModel =
    installedModels === null || installedModels[initialModel]
      ? initialModel
      : DOWNLOAD_MODEL_KEYS.find((model) => installedModels[model]) ?? initialModel;

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
      await requestMicrophoneAccess();
    } catch (error) {
      setMicError(typeof error === 'string' ? error : 'Could not request microphone access.');
    }
  };

  const handleOpenMicSettings = async () => {
    setMicError(null);
    try {
      await openMicrophoneSettings();
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
      await requestAccessibilityPermission();
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

  const handleRequestSystemAudio = async () => {
    setSystemAudioError(null);
    setSystemAudioBusy(true);
    try {
      const access = await requestSystemAudioPermission();
      setSystemAudioStatus(access.permission);
      if (access.needsRelaunch) {
        setSystemAudioError(
          'macOS lists Murmur as allowed, but the permission has not reached this session yet. Quit and reopen Murmur, then check again.',
        );
      }
    } catch (error) {
      setSystemAudioError(typeof error === 'string' ? error : 'Could not check System Audio access.');
    } finally {
      setSystemAudioBusy(false);
    }
  };

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background font-[-apple-system,BlinkMacSystemFont,'Segoe_UI',Roboto,sans-serif]">
      <WindowHeader />
      <div className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto p-8">
      <div className="w-full max-w-lg">
        {/* Persistent wizard progress */}
        <div className="mb-10 flex items-center justify-center">
          <div
            className="flex items-center justify-center gap-2"
            role="progressbar"
            aria-label={`Step ${stepIndex + 1} of ${STEP_ORDER.length}`}
            aria-valuemin={1}
            aria-valuemax={STEP_ORDER.length}
            aria-valuenow={stepIndex + 1}
          >
            {STEP_ORDER.map((s, i) => (
              <span
                key={s}
                aria-hidden="true"
                className={`h-1.5 rounded-full transition-all duration-300 ${
                  i === stepIndex
                    ? 'w-6 bg-primary'
                    : i < stepIndex
                    ? 'w-1.5 bg-primary/60'
                    : 'w-1.5 bg-surface-container-highest '
                }`}
              />
            ))}
          </div>
        </div>

        {step === 'welcome' && (
          <div className="text-center">
            <h1 className="mb-3 text-2xl font-semibold text-on-surface">
              Welcome to Murmur
            </h1>
            <p className="mx-auto mb-3 max-w-md text-sm leading-relaxed text-on-surface-variant">
              Voice-to-text that runs entirely on your Mac. No cloud, no accounts —
              your audio never leaves this machine.
            </p>
            <p className="mx-auto mb-8 max-w-md text-sm leading-relaxed text-on-surface-variant">
              Setup takes about a minute: core macOS permissions, optional System
              Audio access for meetings, and a one-time model download.
            </p>
            <button
              onClick={goNext}
              className="w-full rounded-full bg-[linear-gradient(135deg,var(--murmur-primary),var(--murmur-primary-dim))] px-4 py-3 text-sm font-bold text-on-primary shadow-[0_8px_22px_color-mix(in_srgb,var(--murmur-primary)_20%,transparent)] transition-[filter,transform] hover:brightness-105 active:scale-[0.99]"
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
              <div className="mb-6 px-4 py-3 bg-error/10 border border-error/30 rounded-lg space-y-3">
                <p className="text-sm text-error">
                  Microphone access is denied. Enable Murmur under Privacy &amp;
                  Security → Microphone, then come back — this screen updates
                  automatically.
                </p>
                <button
                  onClick={handleOpenMicSettings}
                  className="w-full rounded-lg border border-error/30 bg-error/10 px-4 py-2 text-sm font-medium text-error transition-colors"
                >
                  Open System Settings
                </button>
                <div>
                  <button
                    onClick={handleResetMic}
                    className="text-xs text-error underline hover:no-underline"
                  >
                    Still not working? Reset the permission
                  </button>
                  <p className="mt-1 text-xs text-error">
                    Clears Murmur's stale Microphone entry so macOS can ask fresh —
                    useful when the toggle is on but recording still fails.
                  </p>
                </div>
              </div>
            ) : (
              <div className="mb-6">
                <button
                  onClick={handleAllowMic}
                  className="w-full py-2.5 px-4 bg-primary hover:bg-primary text-on-primary text-sm font-medium rounded-lg transition-colors"
                >
                  Allow Microphone Access
                </button>
                {micRequested && (
                  <p className="mt-2 text-xs text-on-surface-variant text-center">
                    Waiting for your answer in the macOS dialog…
                  </p>
                )}
              </div>
            )}

            {micError && (
              <p className="mb-4 text-xs text-error">{micError}</p>
            )}

            <WizardNavigationRow
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
                  className="w-full py-2.5 px-4 bg-primary hover:bg-primary text-on-primary text-sm font-medium rounded-lg transition-colors"
                >
                  Grant Accessibility Access
                </button>
                <p className="text-xs text-on-surface-variant text-center">
                  macOS opens System Settings — turn on <strong>Murmur</strong> in the
                  list, then come back. This screen updates automatically.
                </p>
                {axRequested && (
                  <div className="pt-1">
                    <button
                      onClick={handleResetAx}
                      className="text-xs text-on-surface-variant underline hover:no-underline"
                    >
                      Murmur is listed and enabled, but still not detected? Reset the permission
                    </button>
                    <p className="mt-1 text-xs text-on-surface-variant">
                      Clears a stale Accessibility entry (common after reinstalling).
                      You'll need to re-enable Murmur in the list afterward.
                    </p>
                  </div>
                )}
              </div>
            )}

            {axError && (
              <p className="mb-4 text-xs text-error">{axError}</p>
            )}

            <WizardNavigationRow
              onBack={goBack}
              onNext={goNext}
              nextEnabled={axGranted === true}
              nextLabel="Continue"
              skippable={axGranted !== true}
              skipLabel="Skip for now"
            />
          </div>
        )}

        {step === 'systemAudio' && (
          <div>
            <StepHeading
              title="System Audio Access"
              granted={systemAudioStatus === 'granted'}
              subtitle="Optional for Meetings. It lets Murmur capture Mac playback as Them while your microphone remains Me. The permission check creates a short-lived native audio tap only when you press the button."
            />

            {systemAudioStatus === 'granted' ? (
              <GrantedCard label="System Audio access granted" />
            ) : systemAudioStatus === 'unsupported' ? (
              <div className="mb-6 rounded-lg border border-warning/30 bg-warning/10 px-4 py-3 text-sm text-warning">
                Meeting capture requires macOS 14.2 or newer. Dictation is still available.
              </div>
            ) : (
              <div className="mb-6 space-y-3">
                <button
                  type="button"
                  disabled={systemAudioBusy}
                  onClick={() => void handleRequestSystemAudio()}
                  className="w-full rounded-lg bg-primary px-4 py-2.5 text-sm font-medium text-on-primary transition-colors disabled:cursor-wait disabled:opacity-60"
                >
                  {systemAudioBusy ? 'Waiting for macOS…' : systemAudioStatus === 'denied' ? 'Re-check System Audio Access' : 'Allow System Audio Access'}
                </button>
                <p className="text-center text-xs leading-relaxed text-on-surface-variant">
                  macOS may show Murmur under Privacy &amp; Security → Screen &amp; System Audio Recording.
                </p>
                {systemAudioStatus === 'denied' && (
                  <button
                    type="button"
                    onClick={() => void openSystemAudioPreferences()}
                    className="w-full rounded-lg border border-error/30 bg-error/10 px-4 py-2 text-sm font-medium text-error"
                  >
                    Open System Settings
                  </button>
                )}
              </div>
            )}

            {systemAudioError && <p className="mb-4 text-xs text-error">{systemAudioError}</p>}
            <WizardNavigationRow
              onBack={goBack}
              onNext={goNext}
              nextEnabled={systemAudioStatus === 'granted'}
              nextLabel="Continue"
              skippable={systemAudioStatus !== 'granted'}
              skipLabel="Skip Meetings for now"
            />
          </div>
        )}

        {step === 'model' && (
          <div>
            <h1 className="text-xl font-semibold text-on-surface mb-1">
              Transcription Model
            </h1>
            <p className="text-sm text-on-surface-variant mb-6">
              Murmur transcribes with a local model — downloaded once, then everything
              runs offline.
            </p>

            {installedModels === null ? (
              <>
                <div className="h-24" />
                <WizardNavigationRow
                  onBack={goBack}
                  onNext={() => {}}
                  nextEnabled={false}
                  nextLabel="Loading…"
                />
              </>
            ) : (
              <div>
                <ModelDownloadPanel
                  initialModel={preferredModel}
                  installedModels={installedModels}
                  onDownloadingChange={setModelDownloading}
                  renderPrimaryAction={({ label, disabled, onActivate }) => (
                    <WizardNavigationRow
                      onBack={goBack}
                      backDisabled={modelDownloading || disabled}
                      onNext={onActivate}
                      nextEnabled={!disabled}
                      nextLabel={label}
                    />
                  )}
                  onComplete={(model) => {
                    setInstalledModel(model);
                    setModelInstalled(true);
                    setModelDownloading(false);
                    goNext();
                  }}
                />
              </div>
            )}
          </div>
        )}

        {step === 'hotkey' && (
          <div>
            <StepHeading
              title="Recording Shortcut"
              granted
              subtitle="Choose how Murmur listens. You can change both options later in Dictation settings."
            />

            <div className="mb-6 overflow-hidden rounded-xl border border-outline-variant/25 bg-surface-container-lowest">
              <div className="border-b border-outline-variant/15 p-4">
                <p className="mb-2 text-sm font-medium text-on-surface">Recording Trigger</p>
                <div role="group" aria-label="Recording trigger" className="grid grid-cols-3 gap-2">
                  {([
                    ['hold_down', 'Hold Down'],
                    ['double_tap', 'Double-Tap'],
                    ['both', 'Both'],
                  ] as const).map(([value, label]) => (
                    <button
                      key={value}
                      type="button"
                      aria-pressed={selectedRecordingMode === value}
                      onClick={() => setSelectedRecordingMode(value)}
                      className={`rounded-full px-3 py-2 text-xs font-semibold transition-colors ${
                        selectedRecordingMode === value
                          ? 'bg-on-surface text-background'
                          : 'bg-surface-container-high text-on-surface-variant hover:text-on-surface'
                      }`}
                    >
                      {label}
                    </button>
                  ))}
                </div>
              </div>
              <div className="p-4">
                <label htmlFor="onboarding-trigger-key" className="mb-2 block text-sm font-medium text-on-surface">Trigger Key</label>
                <select
                  id="onboarding-trigger-key"
                  value={selectedTriggerKey}
                  onChange={(event) => setSelectedTriggerKey(event.target.value as DoubleTapKey)}
                  className="h-10 w-full rounded-xl border border-outline-variant bg-surface-container-high px-3 text-sm text-on-surface"
                >
                  <option value="shift_l">⇧ Left Shift</option>
                  <option value="alt_l">⌥ Left Option</option>
                  <option value="ctrl_r">⌃ Right Control</option>
                </select>
              </div>
            </div>

            <WizardNavigationRow
              onBack={goBack}
              onNext={goNext}
              nextEnabled
              nextLabel="Continue"
            />
          </div>
        )}

        {step === 'done' && (
          <div>
            <h1 className="text-xl font-semibold text-on-surface mb-1 text-center">
              You're all set
            </h1>
            <p className="text-sm text-on-surface-variant mb-6 text-center">
              Here's how everything looks:
            </p>

            <div className="space-y-2 mb-6">
              <SummaryRow ok={micGranted} label="Microphone" okText="Granted" missingText="Not granted — grant later from the in-app banner or Settings" />
              <SummaryRow ok={axGranted === true} label="Accessibility" okText="Granted" missingText="Not granted — the recording key won't work outside the app" />
              <SummaryRow ok={systemAudioStatus === 'granted'} label="System Audio" okText="Granted for Meetings" missingText="Optional — enable later from Meetings" />
              <SummaryRow ok={modelInstalled === true} label="Model" okText="Installed" missingText="Not verified — the app will ask again if it's missing" />
            </div>

            <div className="mb-6 px-4 py-3 bg-surface-container rounded-lg">
              <p className="text-sm text-on-surface font-medium mb-1">
                Try it out
              </p>
              <p className="text-xs text-on-surface-variant">
                {selectedRecordingMode === 'double_tap' ? 'Double-tap ' : 'Hold '}
                <kbd className="px-1 py-0.5 rounded bg-surface-container-lowest border border-outline-variant/40 font-mono text-[10px]">{KEY_LABELS[selectedTriggerKey]}</kbd>
                {selectedRecordingMode === 'double_tap'
                  ? ' to start recording and tap it once to stop'
                  : selectedRecordingMode === 'both'
                  ? ' and speak, then release (or double-tap to toggle)'
                  : ' and speak, then release'}
                {' '}— your words are transcribed and copied to the clipboard. The
                recording key, auto-paste, and everything else can be changed in
                Settings.
              </p>
            </div>

            <WizardNavigationRow
              onBack={goBack}
              onNext={() => onComplete(installedModel, selectedRecordingMode, selectedTriggerKey)}
              nextEnabled
              nextLabel="Start Using Murmur"
            />
          </div>
        )}
      </div>
      </div>
    </div>
  );
}

function StepHeading({ title, subtitle, granted }: { title: string; subtitle: string; granted: boolean }) {
  return (
    <div className="mb-6">
      <div className="flex items-center gap-2 mb-1">
        <h1 className="text-xl font-semibold text-on-surface">{title}</h1>
        {granted && <CheckIcon />}
      </div>
      <p className="text-sm text-on-surface-variant">{subtitle}</p>
    </div>
  );
}

function GrantedCard({ label }: { label: string }) {
  return (
    <div className="mb-6 px-4 py-3 bg-success/10 border border-success/30 rounded-lg flex items-center gap-2">
      <CheckIcon />
      <span className="text-sm text-success">{label}</span>
    </div>
  );
}

function SummaryRow({ ok, label, okText, missingText }: { ok: boolean; label: string; okText: string; missingText: string }) {
  return (
    <div className="flex items-start gap-2 px-4 py-2.5 bg-surface-container-lowest border border-outline-variant/40 rounded-lg">
      <span className={`mt-1 w-2 h-2 shrink-0 rounded-full ${ok ? 'bg-success' : 'bg-primary'}`} />
      <div className="min-w-0">
        <span className="text-sm font-medium text-on-surface">{label}</span>
        <span className="text-sm text-on-surface-variant"> — {ok ? okText : missingText}</span>
      </div>
    </div>
  );
}

function WizardNavigationRow({
  onBack,
  backDisabled = false,
  onNext,
  nextEnabled,
  nextLabel,
  skippable = false,
  skipLabel = 'Skip',
}: {
  onBack: () => void;
  backDisabled?: boolean;
  onNext: () => void;
  nextEnabled: boolean;
  nextLabel: string;
  skippable?: boolean;
  skipLabel?: string;
}) {
  return (
    <nav aria-label="Setup step actions" className="flex items-center justify-between gap-4">
      <button
        type="button"
        onClick={onBack}
        disabled={backDisabled}
        aria-label="Go back to the previous setup step"
        title={backDisabled ? 'Please wait for the model download to finish' : undefined}
        className="inline-flex items-center gap-1.5 rounded-lg px-2 py-1.5 text-sm font-medium text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:cursor-not-allowed disabled:opacity-40"
      >
        <svg
          aria-hidden="true"
          className="h-4 w-4"
          fill="none"
          stroke="currentColor"
          strokeWidth={2}
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" d="m15 18-6-6 6-6" />
        </svg>
        Back
      </button>
      <div role="group" aria-label="Step actions" className="ml-auto flex items-center gap-3">
        {skippable && (
          <button
            type="button"
            onClick={onNext}
            className="text-xs text-on-surface-variant hover:text-on-surface-variant transition-colors"
          >
            {skipLabel}
          </button>
        )}
        <button
          type="button"
          onClick={onNext}
          disabled={!nextEnabled}
          className="py-2 px-5 bg-primary hover:bg-primary disabled:opacity-40 disabled:cursor-not-allowed text-on-primary text-sm font-medium rounded-lg transition-colors"
        >
          {nextLabel}
        </button>
      </div>
    </nav>
  );
}

function CheckIcon() {
  return (
    <svg className="w-4 h-4 text-success shrink-0" fill="none" stroke="currentColor" strokeWidth={2.5} viewBox="0 0 24 24">
      <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
    </svg>
  );
}
