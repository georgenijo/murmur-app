import { describe, expect, it } from 'vitest';
import {
  audioDeviceSelectOptions,
  followSystemDefaultOptionLabel,
  migrateLegacyMicrophoneId,
  parseAudioInputInventory,
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

  it('makes automatic selection explicit and identifies the live macOS default', () => {
    expect(followSystemDefaultOptionLabel(devices, 'USB-A')).toBe(
      'Follow macOS Default — Studio Mic',
    );
    expect(followSystemDefaultOptionLabel(devices, null)).toBe('Follow macOS Default');
    expect(followSystemDefaultOptionLabel(devices, 'missing')).toBe('Follow macOS Default');
  });

  it('disambiguates duplicate names in the resolved default label', () => {
    const duplicates = [...devices, { id: 'USB-B', name: 'Studio Mic' }];
    expect(followSystemDefaultOptionLabel(duplicates, 'USB-B')).toBe(
      'Follow macOS Default — Studio Mic (USB-B)',
    );
  });
});

describe('parseAudioInputInventory', () => {
  const available = {
    schemaVersion: 1,
    revision: 4,
    status: 'available',
    devices: [{ id: 'uid-1', name: 'Studio Mic' }],
    defaultInputId: 'uid-1',
    errorCode: null,
  };

  it('accepts the exact v1 contract', () => {
    expect(parseAudioInputInventory(available)).toEqual(available);
  });

  it('accepts exact UTF-8 byte boundaries for stable IDs and display names', () => {
    const id = 'x'.repeat(4096);
    const name = '🎙'.repeat(128);
    expect(parseAudioInputInventory({
      ...available,
      devices: [{ id, name }],
      defaultInputId: id,
    })?.devices[0]).toEqual({ id, name });
  });

  it.each([
    { ...available, schemaVersion: 2 },
    { ...available, revision: -1 },
    { ...available, extra: true },
    { ...available, devices: [{ id: 'uid-1', name: 'Mic', extra: true }] },
    { ...available, devices: Array.from({ length: 257 }, (_, index) => ({ id: `uid-${index}`, name: 'Mic' })) },
    { ...available, devices: [{ id: 'x'.repeat(4097), name: 'Mic' }], defaultInputId: null },
    { ...available, devices: [{ id: 'uid-1', name: '🎙'.repeat(129) }] },
    { ...available, status: 'available', errorCode: 'enumerationFailed' },
    { ...available, status: 'stale', errorCode: null },
    { ...available, status: 'stale', errorCode: 'captureActive' },
    { ...available, status: 'unavailable', errorCode: 'notInitialized' },
  ])('rejects malformed or semantically impossible payload %#', (payload) => {
    expect(parseAudioInputInventory(payload)).toBeNull();
  });
});
