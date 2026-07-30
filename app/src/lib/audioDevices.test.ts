import { describe, expect, it } from 'vitest';
import {
  audioDeviceSelectOptions,
  migrateLegacyMicrophoneId,
  selectedDeviceExists,
} from './audioDevices';

const devices = [
  { id: 'BuiltInMicrophoneDevice', name: 'MacBook Microphone' },
  { id: 'USB-A', name: 'Studio Mic' },
];

describe('audio device persistence', () => {
  it('preserves stable IDs and the system default sentinel', () => {
    expect(migrateLegacyMicrophoneId('USB-A', devices)).toBe('USB-A');
    expect(migrateLegacyMicrophoneId('system_default', devices)).toBe('system_default');
  });

  it('migrates an unambiguous legacy display name to its raw device ID', () => {
    expect(migrateLegacyMicrophoneId('Studio Mic', devices)).toBe('USB-A');
  });

  it('fails closed when a legacy display name is ambiguous', () => {
    const duplicates = [...devices, { id: 'USB-B', name: 'Studio Mic' }];
    expect(migrateLegacyMicrophoneId('Studio Mic', duplicates)).toBe('Studio Mic');
    expect(selectedDeviceExists('Studio Mic', duplicates)).toBe(false);
    expect(selectedDeviceExists('Missing Mic', duplicates)).toBe(false);
  });

  it('adds stable IDs only to colliding picker labels', () => {
    const duplicates = [...devices, { id: 'USB-B', name: 'Studio Mic' }];
    expect(audioDeviceSelectOptions(duplicates)).toEqual([
      { value: 'BuiltInMicrophoneDevice', label: 'MacBook Microphone' },
      { value: 'USB-A', label: 'Studio Mic (USB-A)' },
      { value: 'USB-B', label: 'Studio Mic (USB-B)' },
    ]);
  });
});
