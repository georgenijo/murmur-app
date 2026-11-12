import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PermissionsBanner } from './PermissionsBanner';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  checkMicrophonePermissionStatus: vi.fn(),
  requestAccessibilityPermission: vi.fn(),
  resetAccessibilityPermission: vi.fn(),
  resetMicrophonePermission: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mocks.invoke(...args),
}));

vi.mock('../lib/dictation', () => ({
  checkMicrophonePermissionStatus: () => mocks.checkMicrophonePermissionStatus(),
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
  const bannerText = () => container.querySelector('[aria-label="Permission warning"]')?.textContent ?? '';

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

    mocks.resetMicrophonePermission.mockResolvedValueOnce('restarting');
    await clickText('Reset microphone permission…');
    expect(bannerText()).toContain('Murmur is restarting to finish the reset');
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
});
