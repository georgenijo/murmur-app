import { act, useState, type ComponentProps, type ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { OnboardingFlow } from './OnboardingFlow';

const mocks = vi.hoisted(() => ({
  checkAccessibilityPermission: vi.fn(),
  checkMicrophonePermissionStatus: vi.fn(),
  getModelRuntimeCatalog: vi.fn(),
  getSystemAudioPermissionStatus: vi.fn(),
  getCalendarPermissionStatus: vi.fn(),
  getMeetingCalendarEvents: vi.fn(),
  requestCalendarPermission: vi.fn(),
  resetCalendarPermission: vi.fn(),
  openCalendarPreferences: vi.fn(),
  requestSystemAudioPermission: vi.fn(),
  requestMicrophoneAccess: vi.fn().mockResolvedValue(undefined),
  requestAccessibilityPermission: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../lib/dictation', () => ({
  checkAccessibilityPermission: () => mocks.checkAccessibilityPermission(),
  checkMicrophonePermissionStatus: () => mocks.checkMicrophonePermissionStatus(),
  openMicrophoneSettings: vi.fn().mockResolvedValue(undefined),
  requestAccessibilityPermission: () => mocks.requestAccessibilityPermission(),
  requestMicrophoneAccess: () => mocks.requestMicrophoneAccess(),
  resetAccessibilityPermission: vi.fn().mockResolvedValue(undefined),
  resetMicrophonePermission: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../lib/modelRuntime', () => ({
  getModelRuntimeCatalog: () => mocks.getModelRuntimeCatalog(),
}));

vi.mock('../../lib/meetings', () => ({
  getSystemAudioPermissionStatus: () => mocks.getSystemAudioPermissionStatus(),
  openSystemAudioPreferences: vi.fn().mockResolvedValue(undefined),
  requestSystemAudioPermission: () => mocks.requestSystemAudioPermission(),
}));

vi.mock('../../lib/calendar', () => ({
  getCalendarPermissionStatus: () => mocks.getCalendarPermissionStatus(),
  openCalendarPreferences: () => mocks.openCalendarPreferences(),
  requestCalendarPermission: () => mocks.requestCalendarPermission(),
  resetCalendarPermission: () => mocks.resetCalendarPermission(),
  // Onboarding must never ask EventKit for event content.
  getMeetingCalendarEvents: () => mocks.getMeetingCalendarEvents(),
}));

vi.mock('../ModelDownloader', () => ({
  DOWNLOAD_MODEL_KEYS: ['base.en'],
  ModelDownloadPanel: ({
    installedModels,
    onComplete,
    onDownloadingChange,
    renderPrimaryAction,
  }: {
    installedModels: Partial<Record<'base.en', boolean>>;
    onComplete: (model: 'base.en') => void;
    onDownloadingChange: (downloading: boolean) => void;
    renderPrimaryAction: (action: {
      label: string;
      disabled: boolean;
      onActivate: () => void;
      isNavigation: boolean;
    }) => ReactNode;
  }) => {
    const [phase, setPhase] = useState<'idle' | 'downloading' | 'error'>('idle');
    const installed = installedModels['base.en'] === true;
    const startDownload = () => {
      setPhase('downloading');
      onDownloadingChange(true);
    };
    const completeDownload = () => {
      setPhase('idle');
      onDownloadingChange(false);
      onComplete('base.en');
    };
    const failDownload = () => {
      setPhase('error');
      onDownloadingChange(false);
    };
    const label = installed
      ? 'Continue'
      : phase === 'downloading'
      ? 'Downloading…'
      : phase === 'error'
      ? 'Retry download'
      : 'Download';

    return (
      <>
        {renderPrimaryAction({
          label,
          isNavigation: installed,
          disabled: phase === 'downloading',
          onActivate: installed ? () => onComplete('base.en') : startDownload,
        })}
        {phase === 'downloading' && (
          <div aria-label="Model download test controls">
            <button type="button" onClick={failDownload}>Simulate model failure</button>
            <button type="button" onClick={completeDownload}>Complete model download</button>
          </div>
        )}
      </>
    );
  },
}));

vi.mock('../ui/WindowHeader', () => ({
  WindowHeader: () => <div>Window header</div>,
}));

function deferred<T>() {
  let resolve = (_value: T) => {};
  const promise = new Promise<T>((promiseResolve) => {
    resolve = promiseResolve;
  });
  return { promise, resolve };
}

describe('OnboardingFlow', () => {
  let container: HTMLDivElement;
  let root: Root;

  const settle = async () => {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };

  const getButton = (label: string) => {
    const button = Array.from(container.querySelectorAll('button')).find(
      (candidate) => candidate.textContent?.trim() === label,
    );
    expect(button, `Expected a button labeled "${label}"`).toBeDefined();
    return button!;
  };

  const clickButton = async (label: string) => {
    await act(async () => getButton(label).click());
    await settle();
  };

  const renderFlow = async (
    overrides: Partial<ComponentProps<typeof OnboardingFlow>> = {},
  ) => {
    const props: ComponentProps<typeof OnboardingFlow> = {
      initialModel: 'base.en',
      recordingMode: 'hold_down',
      triggerKey: 'shift_l',
      onComplete: vi.fn(),
      ...overrides,
    };
    await act(async () => root.render(<OnboardingFlow {...props} />));
    await settle();
    return props;
  };

  const expectProgress = (step: number) => {
    const progress = container.querySelector<HTMLElement>('[role="progressbar"]');
    expect(progress?.getAttribute('aria-label')).toBe(`Step ${step} of 8`);
    expect(progress?.getAttribute('aria-valuenow')).toBe(String(step));
    return progress!;
  };

  const expectNavigation = (primaryLabel: string, skipLabel?: string) => {
    const row = container.querySelector<HTMLElement>('nav[aria-label="Setup step actions"]');
    expect(row).not.toBeNull();

    const back = row!.querySelector<HTMLButtonElement>(
      '[aria-label="Go back to the previous setup step"]',
    );
    const actions = row!.querySelector<HTMLElement>('[role="group"][aria-label="Step actions"]');
    expect(row!.firstElementChild).toBe(back);
    expect(row!.lastElementChild).toBe(actions);

    const actionButtons = Array.from(actions!.querySelectorAll('button'));
    expect(actionButtons.map((button) => button.textContent?.trim())).toEqual(
      skipLabel ? [skipLabel, primaryLabel] : [primaryLabel],
    );
    expect(actionButtons[actionButtons.length - 1]?.textContent?.trim()).toBe(primaryLabel);
    expect([back, ...actionButtons].every((button) => button?.type === 'button')).toBe(true);
    expect(actionButtons[actionButtons.length - 1]?.className).not.toContain('brightness');
    expect(actionButtons[actionButtons.length - 1]?.className).not.toContain('filter');

    const progress = container.querySelector<HTMLElement>('[role="progressbar"]');
    expect(row!.contains(progress)).toBe(false);
    expect(progress?.contains(row)).toBe(false);

    return { row: row!, back: back!, actions: actions!, actionButtons };
  };

  it('describes local dictation and meeting storage', async () => {
    await renderFlow();
    const copy = container.textContent?.replace(/\s+/g, ' ');
    expect(copy).toContain('Dictation runs on your Mac');
    expect(copy).toContain('Saved history and retained meeting audio stay local');
    expect(copy).not.toContain('No cloud, no accounts');
  });

  const goToSystemAudioStep = async () => {
    await renderFlow();
    await clickButton('Get started');
    await clickButton('Continue');
    await clickButton('Continue');
    expect(container.querySelector('h1')?.textContent).toContain('System audio access');
  };

  const goToModelStep = async () => {
    await goToSystemAudioStep();
    await clickButton('Skip meetings for now');
    expect(container.querySelector('h1')?.textContent).toContain('Calendar access');
    await clickButton('Skip Calendar for now');
    expect(container.querySelector('h1')?.textContent).toContain('Transcription model');
    await settle();
  };

  const goToCalendarStep = async () => {
    await goToSystemAudioStep();
    await clickButton('Skip meetings for now');
    expect(container.querySelector('h1')?.textContent).toContain('Calendar access');
    await settle();
  };

  beforeEach(() => {
    vi.stubEnv('DEV', false);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.checkAccessibilityPermission.mockResolvedValue(true);
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('granted');
    mocks.getModelRuntimeCatalog.mockResolvedValue([
      { modelName: 'base.en', installState: 'installed' },
    ]);
    mocks.getSystemAudioPermissionStatus.mockResolvedValue('unknown');
    mocks.getCalendarPermissionStatus.mockResolvedValue('notDetermined');
    mocks.requestCalendarPermission.mockResolvedValue('granted');
    mocks.resetCalendarPermission.mockResolvedValue(undefined);
    mocks.openCalendarPreferences.mockResolvedValue(undefined);
    mocks.requestSystemAudioPermission.mockResolvedValue({
      permission: 'granted',
      captureReady: true,
      audioFlowing: true,
      needsRelaunch: false,
    });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.clearAllMocks();
    vi.unstubAllEnvs();
  });

  it('uses one action row on every non-welcome step and keeps progress separate', async () => {
    await renderFlow();

    expectProgress(1);
    expect(container.querySelector('nav[aria-label="Setup step actions"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="Go back to the previous setup step"]')).toBeNull();

    await clickButton('Get started');
    expectProgress(2);
    expectNavigation('Continue');

    await clickButton('Continue');
    expectProgress(3);
    expectNavigation('Continue');

    await clickButton('Continue');
    expectProgress(4);
    expectNavigation('Continue', 'Skip meetings for now');

    await clickButton('Skip meetings for now');
    expectProgress(5);
    expectNavigation('Continue', 'Skip Calendar for now');

    await clickButton('Skip Calendar for now');
    expectProgress(6);
    expectNavigation('Continue');

    await clickButton('Continue');
    expectProgress(7);
    expectNavigation('Continue');

    await clickButton('Continue');
    expectProgress(8);
    expectNavigation('Start using Murmur');

    await clickButton('Back');
    expectProgress(7);
    expect(container.querySelector('h1')?.textContent).toContain('Recording shortcut');
  });

  it('keeps Skip secondary while unavailable permission actions stay disabled', async () => {
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('unknown');
    mocks.checkAccessibilityPermission.mockResolvedValue(false);
    await renderFlow();

    await clickButton('Get started');
    let navigation = expectNavigation('Continue', 'Skip for now');
    expect(navigation.actionButtons[0].disabled).toBe(false);
    expect(navigation.actionButtons[1].disabled).toBe(false);

    await clickButton('Skip for now');
    expect(container.querySelector('h1')?.textContent).toContain('Accessibility access');
    navigation = expectNavigation('Open System Settings…', 'Skip for now');
    expect(navigation.actionButtons[0].disabled).toBe(false);
    expect(navigation.actionButtons[1].disabled).toBe(false);

    await clickButton('Skip for now');
    expect(container.querySelector('h1')?.textContent).toContain('System audio access');
    navigation = expectNavigation('Continue', 'Skip meetings for now');
    expect(navigation.actionButtons[0].disabled).toBe(false);
    expect(navigation.actionButtons[1].disabled).toBe(false);

    await clickButton('Skip meetings for now');
    expect(container.querySelector('h1')?.textContent).toContain('Calendar access');
    navigation = expectNavigation('Continue', 'Skip Calendar for now');
    expect(navigation.actionButtons[0].disabled).toBe(false);
    expect(navigation.actionButtons[1].disabled).toBe(false);

    await clickButton('Skip Calendar for now');
    expect(container.querySelector('h1')?.textContent).toContain('Transcription model');
  });

  it('requests microphone access from the single primary action and advances once granted', async () => {
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('notDetermined');
    await renderFlow();
    await clickButton('Get started');
    expectNavigation('Continue', 'Skip for now');
    await clickButton('Continue');
    expect(mocks.requestMicrophoneAccess).toHaveBeenCalledTimes(1);
    expect(container.querySelector('h1')?.textContent).toBe('Microphone access');
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('granted');
    await act(async () => window.dispatchEvent(new Event('focus')));
    await settle();
    await clickButton('Continue');
    expect(container.querySelector('h1')?.textContent).toBe('Accessibility access');
  });

  it('does not request permissions on Enter from a permission heading', async () => {
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('notDetermined');
    mocks.checkAccessibilityPermission.mockResolvedValue(false);
    await renderFlow();
    await clickButton('Get started');
    for (const skip of ['Skip for now', 'Skip for now', 'Skip meetings for now', 'Skip Calendar for now']) {
      const heading = container.querySelector('h1')!;
      expect(document.activeElement).toBe(heading);
      expect(container.querySelector('button[data-default-button]')).toBeNull();
      await act(async () => { heading.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true })); });
      await clickButton(skip);
    }
    expect(mocks.requestMicrophoneAccess).not.toHaveBeenCalled();
    expect(mocks.requestAccessibilityPermission).not.toHaveBeenCalled();
    expect(mocks.requestSystemAudioPermission).not.toHaveBeenCalled();
    expect(mocks.requestCalendarPermission).not.toHaveBeenCalled();
  });

  it('advances on Enter from a heading when Continue is navigation', async () => {
    await renderFlow();
    await clickButton('Get started');
    const heading = container.querySelector('h1')!;
    expect(document.activeElement).toBe(heading);
    await act(async () => { heading.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true })); });
    expect(container.querySelector('h1')?.textContent).toBe('Accessibility access');
  });

  it('requests microphone access only once while the request is pending', async () => {
    const request = deferred<void>();
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('notDetermined');
    mocks.requestMicrophoneAccess.mockReturnValueOnce(request.promise);
    await renderFlow();
    await clickButton('Get started');
    const primary = getButton('Continue');
    await act(async () => { primary.click(); primary.click(); });
    expect(primary.disabled).toBe(true);
    expect(mocks.requestMicrophoneAccess).toHaveBeenCalledTimes(1);
    await act(async () => request.resolve(undefined));
    expect(primary.disabled).toBe(true); // Native request returns before the dialog is answered.
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('denied');
    await act(async () => window.dispatchEvent(new Event('focus')));
    expect(getButton('Open System Settings…').disabled).toBe(false);
  });

  it('offers System Settings as the primary action when microphone access is denied', async () => {
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('denied');
    await renderFlow();
    await clickButton('Get started');
    expectNavigation('Open System Settings…', 'Skip for now');
  });

  it('keeps Back available while the model catalog is loading', async () => {
    mocks.getModelRuntimeCatalog.mockReturnValue(new Promise(() => {}));
    await goToModelStep();

    const navigation = expectNavigation('Loading…');
    expect(navigation.back.disabled).toBe(false);
    expect(navigation.actionButtons[0].disabled).toBe(true);

    await clickButton('Back');
    expect(container.querySelector('h1')?.textContent).toContain('Calendar access');
  });

  it('locks model navigation during download and restores it for retry', async () => {
    mocks.getModelRuntimeCatalog.mockResolvedValue([
      { modelName: 'base.en', installState: 'notInstalled' },
    ]);
    await goToModelStep();

    let navigation = expectNavigation('Download');
    await clickButton('Download');
    navigation = expectNavigation('Downloading…');
    expect(navigation.back.disabled).toBe(true);
    expect(navigation.back.title).toBe('Please wait for the model download to finish');
    expect(navigation.actionButtons[0].disabled).toBe(true);

    await clickButton('Back');
    expect(container.querySelector('h1')?.textContent).toContain('Transcription model');

    await clickButton('Simulate model failure');
    navigation = expectNavigation('Retry download');
    expect(navigation.back.disabled).toBe(false);
    expect(navigation.back.title).toBe('');

    await clickButton('Retry download');
    expect(expectNavigation('Downloading…').back.disabled).toBe(true);
    await clickButton('Complete model download');
    expect(container.querySelector('h1')?.textContent).toContain('Recording shortcut');
    expect(container.textContent).toContain('You can change both options later in Recording settings.');
    expect(container.textContent).not.toContain('Dictation settings');
  });

  it('preserves native focus order and selected shortcut through Back and completion', async () => {
    const onComplete = vi.fn();
    await renderFlow({ onComplete });
    await clickButton('Get started');

    const micNavigation = expectNavigation('Continue');
    micNavigation.back.focus();
    expect(document.activeElement).toBe(micNavigation.back);
    micNavigation.actionButtons[0].focus();
    expect(document.activeElement).toBe(micNavigation.actionButtons[0]);

    await clickButton('Continue');
    await clickButton('Continue');
    await clickButton('Skip meetings for now');
    await clickButton('Skip Calendar for now');
    await clickButton('Continue');

    await clickButton('Double-tap');
    expect(container.querySelector('select')).toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
    await act(async () => Array.from(container.querySelectorAll<HTMLElement>('[role="option"]')).find(option => option.textContent?.includes('Left Option'))!.click());

    await clickButton('Continue');
    await clickButton('Back');
    expect(getButton('Double-tap').getAttribute('aria-pressed')).toBe('true');
    expect(container.querySelector('[role="combobox"]')?.textContent).toContain('Left Option');

    await clickButton('Continue');
    await clickButton('Start using Murmur');
    expect(onComplete).toHaveBeenCalledWith('base.en', 'double_tap', 'alt_l');
  });

  it('offers F1 through F20 and carries a function key into the final hint', async () => {
    const onComplete = vi.fn();
    await renderFlow({ onComplete });
    await clickButton('Get started');
    await clickButton('Continue');
    await clickButton('Continue');
    await clickButton('Skip meetings for now');
    await clickButton('Skip Calendar for now');
    await clickButton('Continue');

    await act(async () => container.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
    const functionOptions = Array.from(container.querySelectorAll<HTMLElement>('[role="option"]')).filter(option => /^F\d+$/.test(option.textContent?.trim() ?? ''));
    expect(functionOptions.map(option => option.textContent?.trim())).toEqual(Array.from({ length: 20 }, (_, index) => `F${index + 1}`));
    await act(async () => functionOptions[11].click());
    expect(container.textContent).toContain('F1–F12 may require Fn');
    expect(container.textContent).toContain("does not suppress the key's normal system or app action");

    await clickButton('Continue');
    expect(container.querySelector('kbd')?.textContent).toBe('F12');
    await clickButton('Start using Murmur');
    expect(onComplete).toHaveBeenCalledWith('base.en', 'hold_down', 'f12');
  });

  it('treats an authorized tap with no audio playing as granted (#638)', async () => {
    mocks.requestSystemAudioPermission.mockResolvedValue({
      permission: 'granted',
      captureReady: true,
      audioFlowing: false,
      needsRelaunch: false,
    });
    await goToSystemAudioStep();

    await clickButton('Continue');

    expect(container.textContent).not.toContain('Meeting audio capture failed');
    expect(container.querySelector('.onboarding-permission-state')?.textContent).toBe('Allowed');
  });

  it('asks for a relaunch when macOS reports access but the tap is refused', async () => {
    mocks.requestSystemAudioPermission.mockResolvedValue({
      permission: 'denied',
      captureReady: false,
      audioFlowing: false,
      needsRelaunch: true,
    });
    await goToSystemAudioStep();

    await clickButton('Continue');

    expect(container.textContent).toContain('Quit and reopen Murmur');
  });

  it('does not touch Calendar before its step and requests access only after a click', async () => {
    await renderFlow();

    expect(mocks.getCalendarPermissionStatus).not.toHaveBeenCalled();
    expect(mocks.requestCalendarPermission).not.toHaveBeenCalled();
    expect(mocks.getMeetingCalendarEvents).not.toHaveBeenCalled();

    await clickButton('Get started');
    await clickButton('Continue');
    await clickButton('Continue');
    expect(mocks.getCalendarPermissionStatus).not.toHaveBeenCalled();

    await clickButton('Skip meetings for now');
    expect(mocks.getCalendarPermissionStatus).toHaveBeenCalledTimes(1);
    expect(mocks.requestCalendarPermission).not.toHaveBeenCalled();
    expect(mocks.getMeetingCalendarEvents).not.toHaveBeenCalled();

    await clickButton('Continue');
    expect(mocks.requestCalendarPermission).toHaveBeenCalledTimes(1);
    expect(mocks.getMeetingCalendarEvents).not.toHaveBeenCalled();
    expect(container.querySelector('.onboarding-permission-state')?.textContent).toBe('Allowed');
  });

  it('keeps Calendar request errors local and content-free', async () => {
    mocks.requestCalendarPermission.mockRejectedValue(
      new Error('private EventKit diagnostic that must not reach the UI'),
    );
    await goToCalendarStep();

    await clickButton('Continue');

    expect(container.textContent).toContain('Could not request Calendar access');
    expect(container.textContent).not.toContain('private EventKit diagnostic');
    expect(mocks.getMeetingCalendarEvents).not.toHaveBeenCalled();
  });

  it('skips Calendar without requesting access or reading events', async () => {
    await goToCalendarStep();

    await clickButton('Skip Calendar for now');
    await act(async () => window.dispatchEvent(new Event('focus')));
    await settle();

    expect(container.querySelector('h1')?.textContent).toContain('Transcription model');
    expect(mocks.getCalendarPermissionStatus).toHaveBeenCalledTimes(1);
    expect(mocks.requestCalendarPermission).not.toHaveBeenCalled();
    expect(mocks.getMeetingCalendarEvents).not.toHaveBeenCalled();
  });

  it('explains manual naming and lets denied Calendar access be opened or reset', async () => {
    mocks.getCalendarPermissionStatus.mockResolvedValue('denied');
    await goToCalendarStep();

    expect(container.textContent).toContain('name meetings manually');
    expect(getButton('Reset Calendar permission…').disabled).toBe(false);

    await clickButton('Open System Settings…');
    expect(mocks.openCalendarPreferences).toHaveBeenCalledTimes(1);

    await clickButton('Reset Calendar permission…');
    expect(mocks.resetCalendarPermission).toHaveBeenCalledTimes(1);
    expect(mocks.requestCalendarPermission).not.toHaveBeenCalled();
    expect(getButton('Continue')).toBeDefined();
  });

  it('finishes a pending Calendar request after leaving and re-entering the step', async () => {
    const request = deferred<'granted'>();
    mocks.requestCalendarPermission.mockReturnValue(request.promise);
    await goToCalendarStep();

    await clickButton('Continue');
    expect(getButton('Continue').disabled).toBe(true);
    await clickButton('Skip Calendar for now');
    await clickButton('Back');
    expect(getButton('Continue').disabled).toBe(true);

    await act(async () => request.resolve('granted'));
    await settle();

    expect(container.querySelector('.onboarding-permission-state')?.textContent).toBe('Allowed');
    expect(expectNavigation('Continue').actionButtons[0].disabled).toBe(false);
  });

  it('finishes a pending Calendar reset after leaving and re-entering the step', async () => {
    const reset = deferred<void>();
    mocks.getCalendarPermissionStatus.mockResolvedValue('denied');
    mocks.resetCalendarPermission.mockReturnValue(reset.promise);
    await goToCalendarStep();

    await clickButton('Reset Calendar permission…');
    expect(getButton('Reset Calendar permission…').disabled).toBe(true);
    await clickButton('Skip Calendar for now');
    await clickButton('Back');
    expect(getButton('Reset Calendar permission…').disabled).toBe(true);

    await act(async () => reset.resolve(undefined));
    await settle();

    expect(getButton('Continue').disabled).toBe(false);
    expect(mocks.requestCalendarPermission).not.toHaveBeenCalled();
  });

  it('refreshes denied Calendar access on focus while its step is visible', async () => {
    mocks.getCalendarPermissionStatus.mockResolvedValue('denied');
    await goToCalendarStep();

    await clickButton('Open System Settings…');
    mocks.getCalendarPermissionStatus.mockResolvedValue('granted');
    await act(async () => window.dispatchEvent(new Event('focus')));
    await settle();

    expect(mocks.getCalendarPermissionStatus).toHaveBeenCalledTimes(2);
    expect(container.querySelector('.onboarding-permission-state')?.textContent).toBe('Allowed');
    expect(expectNavigation('Continue').actionButtons[0].disabled).toBe(false);
    expect(mocks.requestCalendarPermission).not.toHaveBeenCalled();
    expect(mocks.getMeetingCalendarEvents).not.toHaveBeenCalled();
  });
});
