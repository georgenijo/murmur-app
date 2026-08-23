import { act, useState, type ComponentProps, type ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { OnboardingFlow } from './OnboardingFlow';

const mocks = vi.hoisted(() => ({
  checkAccessibilityPermission: vi.fn(),
  checkMicrophonePermissionStatus: vi.fn(),
  getModelRuntimeCatalog: vi.fn(),
  getSystemAudioPermissionStatus: vi.fn(),
  requestSystemAudioPermission: vi.fn(),
}));

vi.mock('../../lib/dictation', () => ({
  checkAccessibilityPermission: () => mocks.checkAccessibilityPermission(),
  checkMicrophonePermissionStatus: () => mocks.checkMicrophonePermissionStatus(),
  openMicrophoneSettings: vi.fn().mockResolvedValue(undefined),
  requestAccessibilityPermission: vi.fn().mockResolvedValue(undefined),
  requestMicrophoneAccess: vi.fn().mockResolvedValue(undefined),
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
      ? 'Downloading...'
      : phase === 'error'
      ? 'Retry Download'
      : 'Download';

    return (
      <>
        {renderPrimaryAction({
          label,
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
    expect(progress?.getAttribute('aria-label')).toBe(`Step ${step} of 7`);
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

    const progress = container.querySelector<HTMLElement>('[role="progressbar"]');
    expect(row!.contains(progress)).toBe(false);
    expect(progress?.contains(row)).toBe(false);

    return { row: row!, back: back!, actions: actions!, actionButtons };
  };

  const goToSystemAudioStep = async () => {
    await renderFlow();
    await clickButton('Get Started');
    await clickButton('Continue');
    await clickButton('Continue');
    expect(container.querySelector('h1')?.textContent).toContain('System Audio Access');
  };

  const goToModelStep = async () => {
    await goToSystemAudioStep();
    await clickButton('Skip Meetings for now');
    expect(container.querySelector('h1')?.textContent).toContain('Transcription Model');
    await settle();
  };

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.checkAccessibilityPermission.mockResolvedValue(true);
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('granted');
    mocks.getModelRuntimeCatalog.mockResolvedValue([
      { modelName: 'base.en', installState: 'installed' },
    ]);
    mocks.getSystemAudioPermissionStatus.mockResolvedValue('unknown');
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
  });

  it('uses one action row on every non-welcome step and keeps progress separate', async () => {
    await renderFlow();

    expectProgress(1);
    expect(container.querySelector('nav[aria-label="Setup step actions"]')).toBeNull();
    expect(container.querySelector('[aria-label="Go back to the previous setup step"]')).toBeNull();

    await clickButton('Get Started');
    expectProgress(2);
    expectNavigation('Continue');

    await clickButton('Continue');
    expectProgress(3);
    expectNavigation('Continue');

    await clickButton('Continue');
    expectProgress(4);
    expectNavigation('Continue', 'Skip Meetings for now');

    await clickButton('Skip Meetings for now');
    expectProgress(5);
    expectNavigation('Continue');

    await clickButton('Continue');
    expectProgress(6);
    expectNavigation('Continue');

    await clickButton('Continue');
    expectProgress(7);
    expectNavigation('Start Using Murmur');

    await clickButton('Back');
    expectProgress(6);
    expect(container.querySelector('h1')?.textContent).toContain('Recording Shortcut');
  });

  it('keeps Skip secondary while unavailable permission actions stay disabled', async () => {
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('unknown');
    mocks.checkAccessibilityPermission.mockResolvedValue(false);
    await renderFlow();

    await clickButton('Get Started');
    let navigation = expectNavigation('Continue', 'Skip for now');
    expect(navigation.actionButtons[0].disabled).toBe(false);
    expect(navigation.actionButtons[1].disabled).toBe(true);

    await clickButton('Skip for now');
    expect(container.querySelector('h1')?.textContent).toContain('Accessibility Access');
    navigation = expectNavigation('Continue', 'Skip for now');
    expect(navigation.actionButtons[0].disabled).toBe(false);
    expect(navigation.actionButtons[1].disabled).toBe(true);

    await clickButton('Skip for now');
    expect(container.querySelector('h1')?.textContent).toContain('System Audio Access');
    navigation = expectNavigation('Continue', 'Skip Meetings for now');
    expect(navigation.actionButtons[0].disabled).toBe(false);
    expect(navigation.actionButtons[1].disabled).toBe(true);

    await clickButton('Skip Meetings for now');
    expect(container.querySelector('h1')?.textContent).toContain('Transcription Model');
  });

  it('keeps Back available while the model catalog is loading', async () => {
    mocks.getModelRuntimeCatalog.mockReturnValue(new Promise(() => {}));
    await goToModelStep();

    const navigation = expectNavigation('Loading…');
    expect(navigation.back.disabled).toBe(false);
    expect(navigation.actionButtons[0].disabled).toBe(true);

    await clickButton('Back');
    expect(container.querySelector('h1')?.textContent).toContain('System Audio Access');
  });

  it('locks model navigation during download and restores it for retry', async () => {
    mocks.getModelRuntimeCatalog.mockResolvedValue([
      { modelName: 'base.en', installState: 'notInstalled' },
    ]);
    await goToModelStep();

    let navigation = expectNavigation('Download');
    await clickButton('Download');
    navigation = expectNavigation('Downloading...');
    expect(navigation.back.disabled).toBe(true);
    expect(navigation.back.title).toBe('Please wait for the model download to finish');
    expect(navigation.actionButtons[0].disabled).toBe(true);

    await clickButton('Back');
    expect(container.querySelector('h1')?.textContent).toContain('Transcription Model');

    await clickButton('Simulate model failure');
    navigation = expectNavigation('Retry Download');
    expect(navigation.back.disabled).toBe(false);
    expect(navigation.back.title).toBe('');

    await clickButton('Retry Download');
    expect(expectNavigation('Downloading...').back.disabled).toBe(true);
    await clickButton('Complete model download');
    expect(container.querySelector('h1')?.textContent).toContain('Recording Shortcut');
  });

  it('preserves native focus order and selected shortcut through Back and completion', async () => {
    const onComplete = vi.fn();
    await renderFlow({ onComplete });
    await clickButton('Get Started');

    const micNavigation = expectNavigation('Continue');
    micNavigation.back.focus();
    expect(document.activeElement).toBe(micNavigation.back);
    micNavigation.actionButtons[0].focus();
    expect(document.activeElement).toBe(micNavigation.actionButtons[0]);

    await clickButton('Continue');
    await clickButton('Continue');
    await clickButton('Skip Meetings for now');
    await clickButton('Continue');

    await clickButton('Double-Tap');
    const triggerKey = container.querySelector<HTMLSelectElement>('#onboarding-trigger-key');
    expect(triggerKey?.labels?.[0]?.textContent).toBe('Trigger Key');
    await act(async () => {
      triggerKey!.value = 'alt_l';
      triggerKey!.dispatchEvent(new Event('change', { bubbles: true }));
    });

    await clickButton('Continue');
    await clickButton('Back');
    expect(getButton('Double-Tap').getAttribute('aria-pressed')).toBe('true');
    expect(container.querySelector<HTMLSelectElement>('#onboarding-trigger-key')?.value).toBe('alt_l');

    await clickButton('Continue');
    await clickButton('Start Using Murmur');
    expect(onComplete).toHaveBeenCalledWith('base.en', 'double_tap', 'alt_l');
  });

  it('treats an authorized tap with no audio playing as granted (#638)', async () => {
    mocks.requestSystemAudioPermission.mockResolvedValue({
      permission: 'granted',
      captureReady: true,
      audioFlowing: false,
      needsRelaunch: false,
    });
    await goToSystemAudioStep();

    await clickButton('Allow System Audio Access');

    expect(container.textContent).not.toContain('Meeting audio capture failed');
    expect(container.textContent).toContain('System Audio access granted');
  });

  it('asks for a relaunch when macOS reports access but the tap is refused', async () => {
    mocks.requestSystemAudioPermission.mockResolvedValue({
      permission: 'denied',
      captureReady: false,
      audioFlowing: false,
      needsRelaunch: true,
    });
    await goToSystemAudioStep();

    await clickButton('Allow System Audio Access');

    expect(container.textContent).toContain('Quit and reopen Murmur');
  });
});
