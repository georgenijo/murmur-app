import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PermissionsBanner } from './PermissionsBanner';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  checkMicrophonePermissionStatus: vi.fn(),
  checkAccessibilityRestartNeeded: vi.fn(),
  restartMurmurForPermission: vi.fn(),
  requestAccessibilityPermission: vi.fn(),
  resetAccessibilityPermission: vi.fn(),
  resetMicrophonePermission: vi.fn(),
}));

const events = vi.hoisted(() => ({ handlers: new Map<string, () => void>() }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: () => void) => {
    events.handlers.set(name, handler);
    return Promise.resolve(() => { events.handlers.delete(name); });
  },
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mocks.invoke(...args),
}));

vi.mock('../lib/dictation', () => ({
  checkMicrophonePermissionStatus: () => mocks.checkMicrophonePermissionStatus(),
  checkAccessibilityRestartNeeded: () => mocks.checkAccessibilityRestartNeeded(),
  restartMurmurForPermission: () => mocks.restartMurmurForPermission(),
  requestAccessibilityPermission: () => mocks.requestAccessibilityPermission(),
  resetAccessibilityPermission: () => mocks.resetAccessibilityPermission(),
  resetMicrophonePermission: () => mocks.resetMicrophonePermission(),
}));

describe('PermissionsBanner', () => {
  let container: HTMLDivElement;
  let root: Root;

  const settle = async () => {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    mocks.invoke.mockImplementation(async (command: string) =>
      command === 'check_accessibility_permission' ? false : undefined);
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('denied');
    mocks.checkAccessibilityRestartNeeded.mockResolvedValue(false);
    mocks.restartMurmurForPermission.mockReset();
    mocks.restartMurmurForPermission.mockResolvedValue('restarting');
    mocks.requestAccessibilityPermission.mockReset();
    mocks.requestAccessibilityPermission.mockResolvedValue('opened_settings');
    mocks.resetAccessibilityPermission.mockResolvedValue('restarting');
    mocks.resetMicrophonePermission.mockResolvedValue('restarting');
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it('lets Home own mic-only denial while retaining other permission warnings', async () => {
    mocks.invoke.mockImplementation(async (command: string) => command === 'check_accessibility_permission' ? true : 'granted');
    await act(async () => root.render(<PermissionsBanner microphoneInHome />));
    await settle();
    expect(container.querySelector('[aria-label="Permission warning"]')).toBeNull();
    mocks.invoke.mockImplementation(async (command: string) => command === 'check_accessibility_permission' ? false : 'granted');
    await act(async () => window.dispatchEvent(new Event('focus')));
    await settle();
    expect(container.querySelector('[aria-label="Permission warning"]')).not.toBeNull();
  });

  it('keeps recovery details open when reset fails and permissions refresh', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.resetMicrophonePermission.mockRejectedValueOnce(new Error('reset failed'));
    await act(async () => root.render(<PermissionsBanner />));
    await settle();

    const details = container.querySelector('details') as HTMLDetailsElement;
    await act(async () => {
      details.open = true;
      details.dispatchEvent(new Event('toggle'));
    });
    const reset = Array.from(container.querySelectorAll('button')).find((button) =>
      button.textContent?.includes('Reset microphone permission…'),
    )!;
    await act(async () => reset.click());
    await settle();

    expect((container.querySelector('details') as HTMLDetailsElement).open).toBe(true);
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      "Couldn't reset the Microphone entry",
    );
  });

  const renderBanner = async () => {
    await act(async () => root.render(<PermissionsBanner />));
    await settle();
  };
  const clickText = async (text: string) => {
    const target = Array.from(container.querySelectorAll('button')).find((button) =>
      button.textContent?.includes(text),
    )!;
    expect(target, text).toBeDefined();
    await act(async () => target.click());
    await settle();
  };
  const bannerText = () =>
    container.querySelector('[aria-label="Permission warning"], [aria-label="Restart pending"]')?.textContent ?? '';

  it('says what the Accessibility button did instead of staying silent', async () => {
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('granted');
    await renderBanner();
    await clickText('Open System Settings…');
    expect(mocks.requestAccessibilityPermission).toHaveBeenCalledTimes(1);
    expect(bannerText()).toContain('Turn on Murmur in System Settings');

    mocks.requestAccessibilityPermission.mockResolvedValueOnce('prompted');
    await clickText('Open System Settings…');
    expect(bannerText()).toContain('Choose Open System Settings in the macOS dialog');

    mocks.requestAccessibilityPermission.mockResolvedValueOnce('restart_deferred');
    await clickText('Open System Settings…');
    expect(bannerText()).toContain('Murmur is busy, so it did not restart');
  });

  it('shows the restarting outcome after an Accessibility or Microphone reset', async () => {
    await renderBanner();
    await clickText('Reset Accessibility permission…');
    expect(bannerText()).toContain('Murmur is restarting to finish the reset');
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it('shows the restarting outcome after a Microphone reset, and when it must wait', async () => {
    await renderBanner();
    mocks.resetMicrophonePermission.mockResolvedValueOnce('restart_pending');
    await clickText('Reset microphone permission…');
    expect(bannerText()).toContain('restarts as soon as your recording, meeting or download finishes');
  });

  it('shows the refusal when a reset is blocked by a recording or meeting', async () => {
    const refusal = 'Finish your recording, meeting or transcription first.';
    mocks.resetAccessibilityPermission.mockRejectedValueOnce(refusal);
    mocks.resetMicrophonePermission.mockRejectedValueOnce(refusal);
    vi.spyOn(console, 'error').mockImplementation(() => {});
    await renderBanner();
    await clickText('Reset Accessibility permission…');
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(refusal);
    expect(bannerText()).not.toContain('restarting');
    await clickText('Reset microphone permission…');
    expect(container.textContent).toContain(refusal);
    expect(bannerText()).not.toContain('restarting');
  });

  const grantedExceptRestart = () => {
    mocks.invoke.mockImplementation(async (command: string) =>
      command === 'check_accessibility_permission' ? true : 'granted');
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('granted');
  };
  const refocus = async () => {
    await act(async () => window.dispatchEvent(new Event('focus')));
    await settle();
  };

  it('shows no restart notice after a plain Accessibility grant', async () => {
    grantedExceptRestart();
    await renderBanner();
    expect(container.querySelector('[aria-label="Permission warning"]')).toBeNull();
    expect(mocks.restartMurmurForPermission).not.toHaveBeenCalled();
  });

  it('shows a restart notice only when this copy cannot use a granted Accessibility, and restarts only on click', async () => {
    grantedExceptRestart();
    mocks.checkAccessibilityRestartNeeded.mockResolvedValue(true);
    await renderBanner();
    expect(bannerText()).toContain("Accessibility is on, but this running copy of Murmur can't use it yet");
    // Nothing happens by itself, however long the notice stays up.
    await settle();
    expect(mocks.restartMurmurForPermission).not.toHaveBeenCalled();
    await clickText('Restart Murmur');
    expect(mocks.restartMurmurForPermission).toHaveBeenCalledTimes(1);
    expect(bannerText()).toContain('Murmur is restarting');
  });

  it('shows the busy refusal when Restart Murmur is blocked, and keeps the notice', async () => {
    grantedExceptRestart();
    mocks.checkAccessibilityRestartNeeded.mockResolvedValue(true);
    mocks.restartMurmurForPermission.mockRejectedValueOnce(
      'Finish your recording, meeting, transcription, download or update first.',
    );
    await renderBanner();
    await clickText('Restart Murmur');
    expect(bannerText()).toContain('Finish your recording');
    expect(bannerText()).not.toContain('is restarting');
    mocks.restartMurmurForPermission.mockResolvedValueOnce('restarting');
    await clickText('Restart Murmur');
    expect(mocks.restartMurmurForPermission).toHaveBeenCalledTimes(2);
  });

  it('clears the restart notice when a later Accessibility query succeeds, and when dismissed', async () => {
    grantedExceptRestart();
    mocks.checkAccessibilityRestartNeeded.mockResolvedValue(true);
    await renderBanner();
    expect(bannerText()).toContain('Restart Murmur');
    mocks.checkAccessibilityRestartNeeded.mockResolvedValue(false);
    await refocus();
    expect(container.querySelector('[aria-label="Permission warning"]')).toBeNull();

    mocks.checkAccessibilityRestartNeeded.mockResolvedValue(true);
    await refocus();
    expect(bannerText()).toContain('Restart Murmur');
    await act(async () => (container.querySelector('button[aria-label="Dismiss"]') as HTMLButtonElement).click());
    expect(container.querySelector('[aria-label="Permission warning"]')).toBeNull();
  });

  it('drops an action note when the permission state changes instead of hiding the new warning', async () => {
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('granted');
    await renderBanner();
    await clickText('Open System Settings…');
    expect(bannerText()).toContain('Turn on Murmur in System Settings');
    // Microphone access is revoked meanwhile: its warning must not stay hidden.
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('denied');
    await refocus();
    expect(bannerText()).toContain('Microphone and Accessibility access are needed');
    expect(bannerText()).not.toContain('Turn on Murmur in System Settings');
  });

  const emit = async (name: string) => {
    const handler = events.handlers.get(name);
    expect(handler, `listener for ${name}`).toBeDefined();
    await act(async () => handler!());
    await settle();
  };

  it('keeps a non-dismissable restart-pending line up when the permission state changes', async () => {
    await renderBanner();
    await clickText('Reset Accessibility permission…');
    const pending = () => container.querySelector('[aria-label="Restart pending"]')?.textContent ?? '';
    expect(pending()).toContain('Murmur is restarting to finish the reset');
    // The state flips (everything granted, then microphone revoked): the line stays.
    grantedExceptRestart();
    await refocus();
    expect(pending()).toContain('Murmur is restarting');
    mocks.checkMicrophonePermissionStatus.mockResolvedValue('denied');
    await refocus();
    expect(pending()).toContain('Murmur is restarting');
    expect(container.querySelector('button[aria-label="Dismiss"]')).toBeNull();
  });

  it('shows the pending line for a restart scheduled elsewhere, and shows the restart notice as soon as the backend says so', async () => {
    grantedExceptRestart();
    await renderBanner();
    expect(container.querySelector('[aria-label="Permission warning"]')).toBeNull();
    // The flag turns on while the window is not focused: no focus event needed.
    mocks.checkAccessibilityRestartNeeded.mockResolvedValue(true);
    await emit('accessibility-restart-needed');
    expect(bannerText()).toContain('Restart Murmur');
    await emit('permission-relaunch-scheduled');
    expect(container.querySelector('[aria-label="Restart pending"]')?.textContent).toContain(
      'Murmur is restarting to finish a permission change',
    );
  });
});
