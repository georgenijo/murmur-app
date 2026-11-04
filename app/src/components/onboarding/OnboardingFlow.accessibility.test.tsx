import { act, StrictMode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { OnboardingFlow } from './OnboardingFlow';

const native = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: native.listen }));

describe('OnboardingFlow accessibility and permission recovery', () => {
  let container: HTMLDivElement;
  let root: Root;
  let micStatus: string;
  let axGranted: boolean;
  let calendarStatus: string;
  let failedCommand: string | undefined;

  const button = (label: string) => {
    const result = Array.from(container.querySelectorAll('button')).find(
      (candidate) => candidate.textContent?.trim() === label,
    );
    expect(result, `Button: ${label}`).toBeDefined();
    return result!;
  };
  const click = async (label: string) => {
    // Match keyboard navigation: the activated control owns focus before it
    // disappears, instead of relying on jsdom's click() to focus a button.
    button(label).focus();
    await act(async () => button(label).click());
  };
  const render = async () => {
    await act(async () => root.render(
      <StrictMode>
        <OnboardingFlow initialModel="base.en" recordingMode="hold_down" triggerKey="shift_l" onComplete={() => {}} />
      </StrictMode>,
    ));
  };
  const expectHeadingFocus = (title: string) => {
    const heading = container.querySelector('h1');
    expect(heading?.textContent?.trim()).toBe(title);
    expect(heading?.tabIndex).toBe(-1);
    expect(document.activeElement).toBe(heading);
  };
  const toStep = async (step: 'microphone' | 'accessibility' | 'systemAudio' | 'calendar') => {
    await render();
    await click('Get started');
    if (step === 'microphone') return;
    await click(micStatus === 'granted' ? 'Continue' : 'Skip for now');
    if (step === 'accessibility') return;
    await click(axGranted ? 'Continue' : 'Skip for now');
    if (step === 'systemAudio') return;
    await click('Skip meetings for now');
  };

  beforeEach(() => {
    vi.stubEnv('DEV', false);
    micStatus = 'granted';
    axGranted = true;
    calendarStatus = 'notDetermined';
    failedCommand = undefined;
    native.listen.mockResolvedValue(() => {});
    native.invoke.mockImplementation(async (command: string) => {
      if (command === failedCommand) throw new Error('Native permission request failed');
      switch (command) {
        case 'check_microphone_permission_status': return micStatus;
        case 'check_accessibility_permission': return axGranted;
        case 'get_system_audio_permission_status': return 'unknown';
        case 'get_calendar_permission_status': return calendarStatus;
        case 'get_model_runtime_catalog': return [{ modelName: 'base.en', installState: 'installed' }];
        case 'get_model_hardware_guidance': return null;
        case 'request_system_audio_permission': return {
          permission: 'granted', captureReady: true, audioFlowing: false, needsRelaunch: false,
        };
        case 'request_calendar_permission': return 'granted';
        case 'request_microphone_access':
        case 'request_accessibility_permission':
        case 'request_microphone_permission':
        case 'open_calendar_preferences':
        case 'reset_microphone_permission':
        case 'reset_accessibility_permission':
        case 'reset_calendar_permission': return undefined;
        default: throw new Error(`Unexpected IPC command: ${command}`);
      }
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
    vi.unstubAllEnvs();
    vi.resetAllMocks();
  });

  it('preserves initial focus, then focuses every heading on forward and backward navigation', async () => {
    const outside = document.createElement('button');
    document.body.appendChild(outside);
    outside.focus();
    await render();
    expect(document.activeElement).toBe(outside);
    outside.remove();

    const steps = [
      ['Get started', 'Microphone access'],
      ['Continue', 'Accessibility access'],
      ['Continue', 'System audio access'],
      ['Skip meetings for now', 'Calendar access'],
      ['Skip Calendar for now', 'Transcription model'],
      ['Continue', 'Recording shortcut'],
      ['Continue', "You're all set"],
    ];
    for (const [action, title] of steps) {
      await click(action);
      expectHeadingFocus(title);
    }
    for (const title of [...steps.slice(0, -1).map(([, title]) => title).reverse(), 'Welcome to Murmur']) {
      await click('Back');
      expectHeadingFocus(title);
    }
  });

  it('announces a polled grant without taking focus from the current control', async () => {
    vi.useFakeTimers();
    micStatus = 'notDetermined';
    await toStep('microphone');
    const allow = button('Allow microphone access…');
    allow.focus();
    const status = container.querySelector('[role="status"]');
    expect(status?.textContent).toBe('Microphone access not yet granted.');
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(document.activeElement).toBe(allow);
    // An unrelated control survives the next grant render.
    const back = button('Back');
    back.focus();
    micStatus = 'granted';
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(document.activeElement).toBe(back);
    expect(container.querySelector('[role="status"]')).toBe(status);
    expect(status?.textContent).toBe('Microphone access granted.');
    expect(status?.getAttribute('aria-atomic')).toBe('true');
    await act(async () => { window.dispatchEvent(new Event('focus')); });
    expect(document.activeElement).toBe(back);
    for (const icon of container.querySelectorAll('svg')) expect(icon.getAttribute('aria-hidden')).toBe('true');
  });

  it.each([
    ['microphone', 'request_microphone_access', 'Allow microphone access…', 'Could not request microphone access.'],
    ['accessibility', 'request_accessibility_permission', 'Grant accessibility access…', 'Could not open System Settings.'],
    ['systemAudio', 'request_system_audio_permission', 'Allow system audio access', 'Could not check System Audio access.'],
    ['calendar', 'request_calendar_permission', 'Allow Calendar access', 'Could not request Calendar access.'],
  ] as const)('announces %s permission errors as alerts', async (step, command, action, message) => {
    micStatus = 'notDetermined';
    axGranted = false;
    failedCommand = command;
    await toStep(step);
    await click(action);
    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain(message);
    expect(document.activeElement).toBe(button(action));
  });

  it('announces optional permission grants through the persistent status region', async () => {
    await toStep('systemAudio');
    await click('Allow system audio access');
    expect(container.querySelector('[role="status"]')?.textContent).toBe('System audio access granted.');
    await click('Continue');
    await click('Allow Calendar access');
    expect(container.querySelector('[role="status"]')?.textContent).toBe('Calendar access granted.');
  });

  it('hides bundle-targeted Reset actions in dev and guides recovery for the terminal identity', async () => {
    vi.stubEnv('DEV', true);
    micStatus = 'denied';
    axGranted = false;
    calendarStatus = 'denied';
    await toStep('microphone');
    const expectDevRecovery = () => {
      expect(container.textContent).toContain('terminal app that launched');
      expect(container.textContent).toContain('restart the dev server');
      expect(container.textContent).toContain("terminal's permission off and on");
      expect(Array.from(container.querySelectorAll('button')).some((node) => /reset/i.test(node.textContent ?? ''))).toBe(false);
    };
    expectDevRecovery();
    await click('Skip for now');
    await click('Grant accessibility access…');
    expectDevRecovery();
    await click('Skip for now');
    await click('Skip meetings for now');
    expectDevRecovery();
    expect(native.invoke.mock.calls.some(([command]) => /^reset_.*_permission$/.test(command))).toBe(false);
  });

  it('retains working bundle-targeted Reset actions in built apps', async () => {
    micStatus = 'denied';
    axGranted = false;
    calendarStatus = 'denied';
    await toStep('microphone');
    await click('Reset permission…');
    expect(native.invoke).toHaveBeenCalledWith('reset_microphone_permission');
    await click('Skip for now');
    await click('Grant accessibility access…');
    await click('Murmur is listed and enabled, but still not detected? Reset the permission');
    expect(native.invoke).toHaveBeenCalledWith('reset_accessibility_permission');
    await click('Skip for now');
    await click('Skip meetings for now');
    await click('Reset Calendar permission…');
    expect(native.invoke).toHaveBeenCalledWith('reset_calendar_permission');
  });
});
