import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PermissionsBanner } from './PermissionsBanner';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  checkMicrophonePermissionStatus: vi.fn(),
  resetAccessibilityPermission: vi.fn(),
  resetMicrophonePermission: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mocks.invoke(...args),
}));

vi.mock('../lib/dictation', () => ({
  checkMicrophonePermissionStatus: () => mocks.checkMicrophonePermissionStatus(),
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
    mocks.resetAccessibilityPermission.mockResolvedValue(undefined);
    mocks.resetMicrophonePermission.mockResolvedValue(undefined);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
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
      button.textContent?.includes('Reset Microphone permission'),
    )!;
    await act(async () => reset.click());
    await settle();

    expect((container.querySelector('details') as HTMLDetailsElement).open).toBe(true);
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      "Couldn't reset the Microphone entry",
    );
  });
});
