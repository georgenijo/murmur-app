import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { OnboardingFlow } from './OnboardingFlow';

const mocks = vi.hoisted(() => ({
  getModelRuntimeCatalog: vi.fn(),
  requestSystemAudioPermission: vi.fn(),
}));

vi.mock('../../lib/dictation', () => ({
  checkAccessibilityPermission: vi.fn().mockResolvedValue(true),
  checkMicrophonePermissionStatus: vi.fn().mockResolvedValue('granted'),
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
  getSystemAudioPermissionStatus: vi.fn().mockResolvedValue('unknown'),
  openSystemAudioPreferences: vi.fn().mockResolvedValue(undefined),
  requestSystemAudioPermission: () => mocks.requestSystemAudioPermission(),
}));

vi.mock('../ModelDownloader', () => ({
  DOWNLOAD_MODEL_KEYS: ['base.en'],
  ModelDownloadPanel: ({
    onComplete,
    onDownloadingChange,
  }: {
    onComplete: (model: 'base.en') => void;
    onDownloadingChange: (downloading: boolean) => void;
  }) => (
    <>
      <button type="button" onClick={() => onDownloadingChange(true)}>
        Start model download
      </button>
      <button type="button" onClick={() => onComplete('base.en')}>
        Continue with model
      </button>
    </>
  ),
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

  const clickButton = async (label: string) => {
    const button = Array.from(container.querySelectorAll('button')).find(
      (candidate) => candidate.textContent?.trim() === label,
    );
    expect(button, `Expected a button labeled "${label}"`).toBeDefined();
    await act(async () => button!.click());
    await settle();
  };

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.getModelRuntimeCatalog.mockResolvedValue([
      { modelName: 'base.en', installState: 'installed' },
    ]);
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

  it('keeps Back available through the final step without replacing the primary action', async () => {
    await act(async () => root.render(
      <OnboardingFlow
        initialModel="base.en"
        recordingMode="hold_down"
        triggerKey="shift_l"
        onComplete={vi.fn()}
      />,
    ));
    await settle();

    expect(container.querySelector('[aria-label="Go back to the previous setup step"]')).toBeNull();

    await clickButton('Get Started');
    expect(container.querySelector('[aria-label="Go back to the previous setup step"]')).not.toBeNull();

    await clickButton('Continue');
    expect(container.textContent).toContain('Accessibility Access');

    await clickButton('Continue');
    expect(container.textContent).toContain('System Audio Access');

    await clickButton('Skip Meetings for now');
    expect(container.textContent).toContain('Transcription Model');

    await clickButton('Start model download');
    const backButton = container.querySelector<HTMLButtonElement>(
      '[aria-label="Go back to the previous setup step"]',
    );
    expect(backButton?.disabled).toBe(true);

    await clickButton('Continue with model');
    expect(container.textContent).toContain('Recording Shortcut');

    await clickButton('Continue');
    expect(container.textContent).toContain("You're all set");
    expect(container.textContent).toContain('Start Using Murmur');
    expect(container.querySelector('[aria-label="Go back to the previous setup step"]')).not.toBeNull();

    await clickButton('Back');
    expect(container.textContent).toContain('Recording Shortcut');
  });

  const goToSystemAudioStep = async () => {
    await act(async () => root.render(
      <OnboardingFlow
        initialModel="base.en"
        recordingMode="hold_down"
        triggerKey="shift_l"
        onComplete={vi.fn()}
      />,
    ));
    await settle();
    await clickButton('Get Started');
    await clickButton('Continue');
    await clickButton('Continue');
    expect(container.textContent).toContain('System Audio Access');
  };

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
