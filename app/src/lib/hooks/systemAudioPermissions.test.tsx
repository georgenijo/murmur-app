import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PermissionsBanner } from '../../components/PermissionsBanner';
import { OnboardingFlow } from '../../components/onboarding/OnboardingFlow';
import { useMeetings } from './useMeetings';
import { DEFAULT_SETTINGS } from '../settings';
import { IDLE_MEETING_STATUS, type SystemAudioAccess } from '../meetings';
import deniedAccess from '../fixtures/system-audio-access.json';

const native = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: native.invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, callback: (event: { payload: unknown }) => void) => {
    native.listeners.set(name, callback);
    return () => native.listeners.delete(name);
  }),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
const grantedAccess: SystemAudioAccess = {
  permission: 'granted', captureReady: true, audioFlowing: false, needsRelaunch: false,
};

describe('System Audio native permission contract', () => {
  let root: Root;
  let container: HTMLDivElement;
  let passiveStatus: string | Promise<string>;
  let probeResult: SystemAudioAccess;

  beforeEach(() => {
    vi.clearAllMocks();
    native.listeners.clear();
    passiveStatus = 'unknown';
    probeResult = grantedAccess;
    native.invoke.mockImplementation(async (command: string) => {
      switch (command) {
        case 'check_accessibility_permission': return true;
        case 'check_microphone_permission_status': return 'granted';
        case 'get_system_audio_permission_status': return passiveStatus;
        case 'request_system_audio_permission': return probeResult;
        case 'get_meeting_status': return IDLE_MEETING_STATUS;
        case 'list_meetings': return { sessions: [], searchMatches: {}, total: 0, offset: 0, limit: 50 };
        default: return undefined;
      }
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });
  const click = async (label: string) => {
    const button = [...container.querySelectorAll('button')].find((item) => item.textContent?.trim() === label);
    expect(button, label).toBeDefined();
    await act(async () => button!.click());
  };
  const emit = async (payload: unknown) => {
    expect(native.listeners.has('system-audio-permission-changed')).toBe(true);
    await act(async () => native.listeners.get('system-audio-permission-changed')!({ payload }));
  };
  const enterSystemAudio = async () => {
    await act(async () => root.render(<OnboardingFlow initialModel="base.en" recordingMode="hold_down" triggerKey="shift_l" onComplete={vi.fn()} />));
    await click('Get started');
    await click('Continue');
    await click('Continue');
  };

  it('shows, restores after dismissal, and clears the denial banner using the Rust object payload', async () => {
    await act(async () => root.render(<PermissionsBanner />));
    expect(container.textContent).toBe('');
    await emit(deniedAccess);
    expect(container.textContent).toContain('System Audio access was denied');
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Dismiss"]')!.click());
    expect(container.textContent).toBe('');
    await emit(deniedAccess);
    expect(container.textContent).toContain('System Audio access was denied');
    await emit(grantedAccess);
    expect(container.textContent).toBe('');
  });

  it('does not overwrite a newer denial event with a late passive banner read', async () => {
    const pending = deferred<string>();
    passiveStatus = pending.promise;
    await act(async () => root.render(<PermissionsBanner />));
    await emit(deniedAccess);
    await act(async () => pending.resolve('granted'));
    expect(container.textContent).toContain('System Audio access was denied');
  });

  it('updates the real Meetings hook from permission events and ignores late cached initialization', async () => {
    let current!: ReturnType<typeof useMeetings>;
    function Harness() { current = useMeetings(DEFAULT_SETTINGS); return null; }
    const pending = deferred<string>();
    passiveStatus = pending.promise;
    await act(async () => root.render(<Harness />));
    await emit(grantedAccess);
    expect(current.permission).toBe('granted');
    expect(current.access).toEqual(grantedAccess);
    await act(async () => pending.resolve('unknown'));
    expect(current.permission).toBe('granted');
    await emit(deniedAccess);
    expect(current.permission).toBe('denied');
    expect(current.access).toEqual(deniedAccess);
    await act(async () => root.unmount());
    expect(native.listeners.has('system-audio-permission-changed')).toBe(false);
    root = createRoot(container);
  });

  it('keeps an explicit re-check after Granted and shows a later denial without a passive probe', async () => {
    passiveStatus = 'granted';
    await enterSystemAudio();
    expect(container.textContent).toContain('System audio access granted');
    probeResult = { permission: 'denied', captureReady: false, audioFlowing: false, needsRelaunch: true };
    await click('Re-check system audio access');
    expect(native.invoke).toHaveBeenCalledWith('request_system_audio_permission');
    expect(container.textContent).not.toContain('System audio access granted');
    expect(container.textContent).toContain('Quit and reopen Murmur');
    expect(container.textContent).toContain('Open System Settings');
    expect(native.invoke.mock.calls.filter(([command]) => command === 'get_system_audio_permission_status')).toHaveLength(1);
  });

  it('ignores a late cached step-entry read after an explicit permission probe', async () => {
    const pending = deferred<string>();
    passiveStatus = pending.promise;
    await enterSystemAudio();
    await click('Allow system audio access');
    expect(container.textContent).toContain('System audio access granted');
    await act(async () => pending.resolve('unknown'));
    expect(container.textContent).toContain('System audio access granted');
  });
});
