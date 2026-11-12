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
  let accessibilityGate: Promise<void> | null;
  let accessibilityOutcome: string | undefined;
  let resetRefusal: string | undefined;

  const button = (label: string) => {
    const result = Array.from(container.querySelectorAll('button')).find(
      (candidate) => label === 'Download' ? candidate.textContent?.trim().startsWith('Download ·') : candidate.textContent?.trim() === label,
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
    await click('Skip for now');
  };

  beforeEach(() => {
    vi.stubEnv('DEV', false);
    micStatus = 'granted';
    axGranted = true;
    calendarStatus = 'notDetermined';
    failedCommand = undefined;
    accessibilityGate = null;
    accessibilityOutcome = undefined;
    resetRefusal = undefined;
    window.localStorage.clear();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    native.listen.mockResolvedValue(() => {});
    native.invoke.mockImplementation(async (command: string) => {
      if (command === failedCommand) throw new Error('Native permission request failed');
      if (resetRefusal && /^reset_(accessibility|microphone)_permission$/.test(command)) throw resetRefusal;
      switch (command) {
        case 'check_microphone_permission_status': return micStatus;
        case 'check_accessibility_permission': {
          const gate = accessibilityGate;
          if (gate) await gate;
          return axGranted;
        }
        case 'get_system_audio_permission_status': return 'unknown';
        case 'get_calendar_permission_status': return calendarStatus;
        case 'get_model_runtime_catalog': return [{ modelName: 'base.en', installState: 'installed' }];
        case 'get_model_hardware_guidance': return null;
        case 'request_system_audio_permission': return {
          permission: 'granted', captureReady: true, audioFlowing: false, needsRelaunch: false,
        };
        case 'request_calendar_permission': return 'granted';
        case 'show_main_window':
        case 'request_accessibility_permission':
        case 'reset_accessibility_permission': return accessibilityOutcome;
        case 'request_microphone_access':
        case 'request_microphone_permission':
        case 'open_calendar_preferences':
        case 'reset_microphone_permission': return 'restarting';
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
    vi.restoreAllMocks();
    vi.resetAllMocks();
  });

  it('focuses Welcome at mount, then every heading on forward and backward navigation', async () => {
    const outside = document.createElement('button');
    document.body.appendChild(outside);
    outside.focus();
    await render();
    expectHeadingFocus('Welcome to Murmur');
    outside.remove();

    const steps = [
      ['Get started', 'Microphone access'],
      ['Continue', 'Accessibility access'],
      ['Continue', 'System audio access'],
      ['Skip for now', 'Calendar access'],
      ['Skip for now', 'Speech model'],
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

  const advance = async (milliseconds: number) => {
    await act(async () => { await vi.advanceTimersByTimeAsync(milliseconds); });
  };
  const refresh = async () => {
    await act(async () => { window.dispatchEvent(new Event('focus')); });
  };
  const requestCount = () => native.invoke.mock.calls.filter(([command]) => command === 'request_microphone_access').length;
  const expectStep = (title: string) => expect(container.querySelector('h1')?.textContent).toBe(title);

  it('Return on the initially focused Welcome heading starts setup', async () => {
    await render();
    expectHeadingFocus('Welcome to Murmur');
    await act(async () => {
      document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    });
    expectHeadingFocus('Microphone access');
  });

  it('auto-requests the mic once after settling, preserves the native dialog wait, and does not repeat on revisit', async () => {
    vi.useFakeTimers();
    micStatus = 'notDetermined';
    await toStep('microphone');
    await advance(599);
    expect(requestCount()).toBe(0);
    await advance(1);
    expect(requestCount()).toBe(1);
    expect(container.textContent).toContain('Waiting for your answer in the macOS dialog…');
    expect(button('Continue').disabled).toBe(true);
    expect(Array.from(container.querySelectorAll('button')).some(node => node.textContent === 'Skip for now')).toBe(false);
    await refresh();
    await advance(2000);
    expect(requestCount()).toBe(1);
    await click('Back');
    await click('Get started');
    await advance(1000);
    expect(requestCount()).toBe(1);
    micStatus = 'denied';
    await refresh();
    expect(button('Skip for now').disabled).toBe(false);
    expect(button('Open System Settings…').disabled).toBe(false);
  });

  it('re-enables mic actions when no macOS answer arrives', async () => {
    vi.useFakeTimers();
    micStatus = 'notDetermined';
    await toStep('microphone');
    await advance(600);
    expect(requestCount()).toBe(1);
    await advance(14_999);
    expect(button('Continue').disabled).toBe(true);
    await advance(1);
    expect(container.textContent).not.toContain('Waiting for your answer in the macOS dialog…');
    expect(button('Continue').disabled).toBe(false);
    expect(button('Skip for now').disabled).toBe(false);
    expect(requestCount()).toBe(1);
  });

  it.each(['granted', 'denied', 'unknown'])('never auto-requests a %s microphone', async status => {
    vi.useFakeTimers();
    micStatus = status;
    await toStep('microphone');
    await advance(2000);
    expect(requestCount()).toBe(0);
    expectStep('Microphone access');
    if (status === 'unknown') {
      await click('Continue');
      expect(requestCount()).toBe(1);
      expect(button('Skip for now').disabled).toBe(false);
    }
  });

  it('cancels the mic settle timer when leaving the step and on unmount', async () => {
    vi.useFakeTimers();
    micStatus = 'notDetermined';
    await toStep('microphone');
    await advance(300);
    await click('Back');
    await advance(600);
    expect(requestCount()).toBe(0);
    await click('Get started');
    await act(async () => root.render(<div />));
    await advance(600);
    expect(requestCount()).toBe(0);
  });

  it.each(['microphone', 'accessibility'] as const)('shows and announces a fresh %s grant before advancing exactly once', async step => {
    vi.useFakeTimers();
    micStatus = step === 'microphone' ? 'notDetermined' : 'granted';
    axGranted = false;
    await toStep(step);
    if (step === 'microphone') await advance(600);
    if (step === 'microphone') micStatus = 'granted';
    else axGranted = true;
    await refresh();
    const title = step === 'microphone' ? 'Microphone access' : 'Accessibility access';
    expectStep(title);
    expect(container.querySelector('[role="status"]')?.textContent).toBe(`${title} granted.`);
    expect(container.querySelector('[data-fresh="true"]')?.textContent).toBe('Allowed');
    await advance(899);
    expectStep(title);
    await refresh(); // A repeated grant must not restart the timer.
    await advance(1);
    expectStep(step === 'microphone' ? 'Accessibility access' : 'System audio access');
    await advance(2000);
    expectStep(step === 'microphone' ? 'Accessibility access' : 'System audio access');
  });

  it('does not auto-request the mic on a revisit after leaving before the settle delay', async () => {
    vi.useFakeTimers();
    micStatus = 'notDetermined';
    await toStep('microphone');
    await advance(300);
    await click('Back');
    await click('Get started');
    await advance(2000);
    expect(requestCount()).toBe(0);
    expect(button('Continue').disabled).toBe(false);
    await click('Continue');
    expect(requestCount()).toBe(1);
  });

  it('does not auto-advance Accessibility granted between the last poll and arrival', async () => {
    vi.useFakeTimers();
    axGranted = false;
    await toStep('microphone');
    await refresh(); // Accessibility observed as not granted on an earlier step.
    axGranted = true;
    await click('Continue');
    await refresh();
    await advance(2000);
    expectStep('Accessibility access');
    expect(container.querySelector('[data-fresh="true"]')).toBeNull();
    expect(button('Continue').hasAttribute('data-default-button')).toBe(true);
  });

  it.each(['microphone', 'accessibility'] as const)('does not auto-advance an already granted %s on arrival', async step => {
    vi.useFakeTimers();
    await toStep(step);
    await advance(2000);
    expectStep(step === 'microphone' ? 'Microphone access' : 'Accessibility access');
    expect(button('Continue').hasAttribute('data-default-button')).toBe(true);
    expect(container.querySelector('[data-fresh="true"]')).toBeNull();
  });

  it.each(['Back', 'Continue'] as const)('manual %s cancels a pending automatic advance, including revisits', async action => {
    vi.useFakeTimers();
    axGranted = false;
    await toStep('accessibility');
    axGranted = true;
    await refresh();
    await advance(400);
    await click(action);
    if (action === 'Back') await click('Continue');
    await advance(1200);
    expectStep(action === 'Back' ? 'Accessibility access' : 'System audio access');
  });

  it('a grant detected after Skip does not schedule navigation on another step', async () => {
    vi.useFakeTimers();
    axGranted = false;
    await toStep('accessibility');
    await click('Skip for now');
    axGranted = true;
    await refresh();
    await advance(2000);
    expectStep('System audio access');
  });

  it.each([false, true])('foregrounds a fresh Accessibility grant only when unfocused (focused=%s)', async focused => {
    vi.useFakeTimers();
    vi.mocked(document.hasFocus).mockReturnValue(focused);
    axGranted = false;
    await toStep('accessibility');
    expect(native.invoke).not.toHaveBeenCalledWith('request_accessibility_permission');
    await click('Open System Settings…');
    axGranted = true;
    await advance(1000); // The interval detects the System Settings round-trip.
    expect(native.invoke.mock.calls.filter(([command]) => command === 'show_main_window')).toHaveLength(focused ? 0 : 1);
    await advance(900);
    expectStep('System audio access');
  });

  it.each(['Open System Settings…', 'Reset permission…'])('foregrounds a denied microphone grant after %s', async action => {
    vi.useFakeTimers();
    vi.mocked(document.hasFocus).mockReturnValue(false);
    micStatus = 'denied';
    await toStep('microphone');
    await click(action);
    micStatus = 'granted';
    await advance(1000);
    expect(native.invoke).toHaveBeenCalledWith('show_main_window');
    await advance(900);
    expectStep('Accessibility access');
  });

  it('keeps a denied microphone baseline when Settings grants before the entry probe finishes', async () => {
    vi.useFakeTimers();
    micStatus = 'denied';
    await render();
    let release = () => {};
    accessibilityGate = new Promise<void>(resolve => { release = resolve; });
    await click('Get started'); // The entry probe now waits on Accessibility.
    await click('Open System Settings…');
    micStatus = 'granted';
    accessibilityGate = null;
    await refresh(); // Supersedes the pending entry probe.
    expect(container.querySelector('[data-fresh="true"]')?.textContent).toBe('Allowed');
    expect(container.querySelector('[role="status"]')?.textContent).toBe('Microphone access granted.');
    await act(async () => release());
    await advance(900);
    expectStep('Accessibility access');
  });

  it.each(['systemAudio', 'calendar'] as const)('auto-advances a fresh optional %s grant after an explicit request', async step => {
    vi.useFakeTimers();
    await toStep(step);
    expect(native.invoke).not.toHaveBeenCalledWith(step === 'systemAudio' ? 'request_system_audio_permission' : 'request_calendar_permission');
    expect(container.textContent).not.toContain('Continue shows the macOS permission prompt.');
    await click('Continue');
    expect(container.querySelector('[data-fresh="true"]')?.textContent).toBe('Allowed');
    await advance(900);
    expectStep(step === 'systemAudio' ? 'Calendar access' : 'Speech model');
  });

  it.each(['systemAudio', 'calendar'] as const)('does not auto-advance an already granted optional %s entry probe', async step => {
    vi.useFakeTimers();
    if (step === 'calendar') calendarStatus = 'granted';
    if (step === 'systemAudio') {
      // Override just this status, retaining the normal mocks for the rest of setup.
      const invoke = native.invoke.getMockImplementation()!;
      native.invoke.mockImplementation((command: string) => command === 'get_system_audio_permission_status' ? Promise.resolve('granted') : invoke(command));
    }
    await toStep(step);
    await advance(2000);
    expectStep(step === 'systemAudio' ? 'System audio access' : 'Calendar access');
    expect(button('Continue').hasAttribute('data-default-button')).toBe(true);
    expect(container.querySelector('[data-fresh="true"]')).toBeNull();
  });

  it('a revoked grant cancels its pending navigation', async () => {
    vi.useFakeTimers();
    axGranted = false;
    await toStep('accessibility');
    axGranted = true;
    await refresh();
    await advance(400);
    axGranted = false;
    await refresh();
    await advance(900);
    expectStep('Accessibility access');
    expect(container.querySelector('[data-fresh="true"]')).toBeNull();
  });

  it('foreground failure does not block advancement after a grant', async () => {
    vi.useFakeTimers();
    vi.mocked(document.hasFocus).mockReturnValue(false);
    failedCommand = 'show_main_window';
    axGranted = false;
    await toStep('accessibility');
    axGranted = true;
    await refresh();
    await advance(900);
    expectStep('System audio access');
  });

  it('announces a polled grant without taking focus from the current control', async () => {
    vi.useFakeTimers();
    micStatus = 'notDetermined';
    await toStep('microphone');
    const allow = button('Continue');
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
    ['microphone', 'request_microphone_access', 'Continue', 'Could not request microphone access.'],
    ['accessibility', 'request_accessibility_permission', 'Open System Settings…', 'Could not open System Settings.'],
    ['systemAudio', 'request_system_audio_permission', 'Continue', 'Could not check System Audio access.'],
    ['calendar', 'request_calendar_permission', 'Continue', 'Could not request Calendar access.'],
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
    await click('Continue');
    expect(container.querySelector('[role="status"]')?.textContent).toBe('System audio access granted.');
    await click('Continue');
    await click('Continue');
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
    await click('Open System Settings…');
    expectDevRecovery();
    await click('Skip for now');
    await click('Skip for now');
    expectDevRecovery();
    expect(native.invoke.mock.calls.some(([command]) => /^reset_.*_permission$/.test(command))).toBe(false);
  });

  it('tells the user which single surface a first Accessibility request opened', async () => {
    axGranted = false;
    await toStep('accessibility');
    accessibilityOutcome = 'prompted';
    await click('Open System Settings…');
    expect(container.textContent).toContain('Choose Open System Settings in the macOS dialog');
    // The frontend never opens the pane itself: the request command owns that
    // choice, so one click cannot raise the dialog and the pane together.
    const opened = native.invoke.mock.calls.map(([command]) => command);
    expect(opened.filter(command => command === 'request_accessibility_permission')).toHaveLength(1);
    expect(opened).not.toContain('request_microphone_permission');
    expect(opened).not.toContain('open_system_audio_preferences');
  });

  it('explains the restart when resetting a stale Accessibility entry relaunches Murmur', async () => {
    axGranted = false;
    await toStep('accessibility');
    accessibilityOutcome = 'opened_settings';
    await click('Open System Settings…');
    expect(container.textContent).toContain('Turn on Murmur in System Settings');
    accessibilityOutcome = 'restarting';
    await click('Murmur is listed but still not detected? Reset permission…');
    expect(native.invoke).toHaveBeenCalledWith('reset_accessibility_permission');
    expect(container.textContent).toContain('Murmur is restarting to finish the reset');
  });

  it('a microphone reset restarts Murmur instead of opening System Settings', async () => {
    micStatus = 'denied';
    await toStep('microphone');
    await click('Reset permission…');
    expect(native.invoke).toHaveBeenCalledWith('reset_microphone_permission');
    expect(native.invoke).not.toHaveBeenCalledWith('request_microphone_permission');
    expect(container.textContent).toContain('Murmur is restarting to finish the reset');
  });

  it('retains working bundle-targeted Reset actions in built apps', async () => {
    micStatus = 'denied';
    axGranted = false;
    calendarStatus = 'denied';
    await toStep('microphone');
    await act(async () => { window.dispatchEvent(new Event('focus')); });
    expect(container.textContent).not.toContain('terminal app that launched');
    await click('Reset permission…');
    expect(native.invoke).toHaveBeenCalledWith('reset_microphone_permission');
    await click('Skip for now');
    await click('Open System Settings…');
    await click('Murmur is listed but still not detected? Reset permission…');
    expect(native.invoke).toHaveBeenCalledWith('reset_accessibility_permission');
    await click('Skip for now');
    await click('Skip for now');
    await click('Reset Calendar permission…');
    expect(native.invoke).toHaveBeenCalledWith('reset_calendar_permission');
  });

  it('shows a deferred restart without claiming Murmur is restarting', async () => {
    axGranted = false;
    await toStep('accessibility');
    accessibilityOutcome = 'restart_deferred';
    await click('Open System Settings…');
    expect(container.textContent).toContain('Murmur is busy, so it did not restart');
    expect(container.textContent).not.toContain('is restarting to finish the reset');
  });

  it('shows the refusal, not a restart, when a reset is blocked by active work', async () => {
    axGranted = false;
    micStatus = 'denied';
    resetRefusal = 'Finish your recording, meeting or transcription first.';
    await toStep('microphone');
    await click('Reset permission…');
    expect(container.textContent).toContain(resetRefusal);
    expect(container.textContent).not.toContain('Murmur is restarting to finish the reset');
    expect(window.localStorage.getItem('murmur.onboarding.resume')).toBeNull();
    await click('Skip for now');
    await click('Open System Settings…');
    await click('Murmur is listed but still not detected? Reset permission…');
    expect(container.textContent).toContain(resetRefusal);
    expect(container.textContent).not.toContain('Murmur is restarting to finish the reset');
  });

  it('resumes the step that was open after a permission restart, once', async () => {
    axGranted = false;
    await toStep('accessibility');
    accessibilityOutcome = 'restarting';
    await click('Open System Settings…');
    expect(window.localStorage.getItem('murmur.onboarding.resume')).toContain('accessibility');
    // The restarted process mounts a fresh wizard.
    await act(async () => root.unmount());
    root = createRoot(container);
    await render();
    expect(container.querySelector('h1')?.textContent).toBe('Accessibility access');
    // The resume is consumed: a later launch starts at Welcome.
    expect(window.localStorage.getItem('murmur.onboarding.resume')).toBeNull();
    await act(async () => root.unmount());
    root = createRoot(container);
    await render();
    expect(container.querySelector('h1')?.textContent).toBe('Welcome to Murmur');
  });

  it('ignores an old or unknown resume step', async () => {
    window.localStorage.setItem('murmur.onboarding.resume', JSON.stringify({ step: 'accessibility', at: Date.now() - 10 * 60 * 1000 }));
    await render();
    expect(container.querySelector('h1')?.textContent).toBe('Welcome to Murmur');
    await act(async () => root.unmount());
    root = createRoot(container);
    window.localStorage.setItem('murmur.onboarding.resume', JSON.stringify({ step: 'nonsense', at: Date.now() }));
    await render();
    expect(container.querySelector('h1')?.textContent).toBe('Welcome to Murmur');
  });
});
