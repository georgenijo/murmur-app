import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import {
  getSmartAutoMicrophoneStatus,
  parseSmartAutoMicrophoneStatus,
  smartAutoMicrophoneReasonLabel,
  smartAutoProbePolicy,
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
    expect(parseSmartAutoMicrophoneStatus({
      state: 'blocked', message: 'Cooling down.', retryAfterMs: 0,
    })).toBeNull();
    expect(parseSmartAutoMicrophoneStatus({
      state: 'blocked', message: 'Cooling down.', retryAfterMs: 900_001,
    })).toBeNull();
  });

  it('normalizes untimed blocked responses and preserves valid cooldowns', () => {
    expect(parseSmartAutoMicrophoneStatus({ state: 'blocked', message: 'Verify a microphone.' }))
      .toEqual({ state: 'blocked', message: 'Verify a microphone.', retryAfterMs: null });
    expect(parseSmartAutoMicrophoneStatus({
      state: 'blocked', message: 'Switch cooldown.', retryAfterMs: 10_000,
    })).toEqual({ state: 'blocked', message: 'Switch cooldown.', retryAfterMs: 10_000 });
  });

  it('accepts bounded probe progress and longer scheduler backoff', () => {
    expect(parseSmartAutoMicrophoneStatus({
      state: 'probing', deviceId: 'usb', phase: 'verifying', remainingMs: 5_000,
    })).toEqual({ state: 'probing', deviceId: 'usb', phase: 'verifying', remainingMs: 5_000 });
    expect(parseSmartAutoMicrophoneStatus({
      state: 'blocked', message: 'Backoff.', retryAfterMs: 900_000,
    })).toEqual({ state: 'blocked', message: 'Backoff.', retryAfterMs: 900_000 });
    expect(parseSmartAutoMicrophoneStatus({
      state: 'probing', deviceId: 'usb', phase: 'transcribing', remainingMs: 5_000,
    })).toBeNull();
  });

  it('requires separate probe consent in addition to Smart Auto', () => {
    expect(smartAutoProbePolicy({
      microphone: 'system_default',
      disabled: false,
      smartAutoMicrophoneEnabled: true,
      smartAutoProbeEnabled: false,
      smartAutoApprovedDeviceIds: ['usb'],
      smartAutoPreferredDeviceIds: ['usb'],
      smartAutoAllowContinuity: false,
    })).toEqual({ enabled: false });
    expect(smartAutoProbePolicy({
      microphone: 'system_default',
      disabled: false,
      smartAutoMicrophoneEnabled: true,
      smartAutoProbeEnabled: true,
      smartAutoApprovedDeviceIds: ['usb'],
      smartAutoPreferredDeviceIds: ['usb'],
      smartAutoAllowContinuity: false,
    })).toEqual({
      enabled: true,
      request: { approvedDeviceIds: ['usb'], preferredDeviceIds: ['usb'], allowContinuity: false },
    });
  });

  it('does not authorize background checks alongside a pinned microphone', () => {
    expect(smartAutoProbePolicy({
      microphone: 'pinned',
      disabled: false,
      smartAutoMicrophoneEnabled: true,
      smartAutoProbeEnabled: true,
      smartAutoApprovedDeviceIds: ['usb'],
      smartAutoPreferredDeviceIds: ['usb'],
      smartAutoAllowContinuity: false,
    })).toEqual({ enabled: false });
  });

  it('never enables probes before a saved disabled state reaches backend initialization', () => {
    expect(smartAutoProbePolicy({
      disabled: true,
      microphone: 'system_default',
      smartAutoMicrophoneEnabled: true,
      smartAutoProbeEnabled: true,
      smartAutoApprovedDeviceIds: ['usb'],
      smartAutoPreferredDeviceIds: ['usb'],
      smartAutoAllowContinuity: false,
    })).toEqual({ enabled: false });
  });

  it('gives each backend decision reason a user-facing label', () => {
    expect(smartAutoMicrophoneReasonLabel('current_verified')).toBe('already active with recent signal');
    expect(smartAutoMicrophoneReasonLabel('previous_verified_rollback')).toBe('previously active with recent signal');
    expect(smartAutoMicrophoneReasonLabel('preferred_approved')).toBe('your preferred included microphone');
  });
});
