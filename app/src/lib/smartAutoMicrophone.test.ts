import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import {
  getSmartAutoMicrophoneStatus,
  parseSmartAutoMicrophoneStatus,
  smartAutoMicrophoneReasonLabel,
} from './smartAutoMicrophone';

describe('Smart Auto microphone status boundary', () => {
  beforeEach(() => invoke.mockReset());

  it('requests the exact next-capture policy with the Smart Auto envelope', async () => {
    invoke.mockResolvedValue({
      state: 'ready',
      deviceId: 'usb',
      reason: 'current_verified',
      validForMs: 12_000,
    });
    const smartAuto = {
      approvedDeviceIds: ['usb'],
      preferredDeviceIds: ['usb'],
      allowContinuity: false,
    };

    await expect(getSmartAutoMicrophoneStatus(smartAuto)).resolves.toEqual({
      state: 'ready',
      deviceId: 'usb',
      reason: 'current_verified',
      validForMs: 12_000,
    });
    expect(invoke).toHaveBeenCalledWith('get_smart_auto_microphone_status', { smartAuto });
  });

  it('rejects malformed or overlong evidence lifetimes at the IPC boundary', () => {
    expect(parseSmartAutoMicrophoneStatus({
      state: 'ready', deviceId: 'usb', reason: 'current_verified', validForMs: 120_001,
    })).toBeNull();
    expect(parseSmartAutoMicrophoneStatus({
      state: 'ready', deviceId: 'usb', reason: 'not_a_reason', validForMs: 100,
    })).toBeNull();
    expect(parseSmartAutoMicrophoneStatus({ state: 'blocked', message: '' })).toBeNull();
  });

  it('gives each backend decision reason a user-facing label', () => {
    expect(smartAutoMicrophoneReasonLabel('current_verified')).toBe('current verified microphone retained');
    expect(smartAutoMicrophoneReasonLabel('previous_verified_rollback')).toBe('previous verified microphone restored');
    expect(smartAutoMicrophoneReasonLabel('preferred_approved')).toBe('preferred approved microphone');
  });
});
